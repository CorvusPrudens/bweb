//! The JS-side node registry backing bweb's lazy DOM handles.
//!
//! Every managed node is created Rust-side, then immediately *adopted* into a
//! JS `Map` keyed by [`NodeId`]. Handle components ([`Node`], [`Element`],
//! [`HtmlElement`], ...) store only the id and fetch the live `web_sys` value
//! through [`get_node`] on first deref, so structural bookkeeping never has
//! to hold (or pass around) real JS handles.
//!
//! The registry holds *strong* references — unlike the WeakMap it replaced,
//! every despawn path must release its slot via [`remove_node`] (done in
//! `Node::on_remove_hook`) or the node leaks. [`live_nodes`] exists so tests
//! can assert the balance.
//!
//! [`Node`]: crate::dom::html::Node
//! [`Element`]: crate::dom::html::Element
//! [`HtmlElement`]: crate::dom::html::HtmlElement

use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use wasm_bindgen::prelude::*;

/// Index of a managed DOM node in the JS-side registry.
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub(crate) u32);

/// Allocator for [`NodeId`]s. Freed ids are quarantined until the end of the
/// tick before becoming reusable, so an id freed mid-tick can't be re-issued
/// while operations referencing it may still be in flight (ABA guard).
#[derive(Resource, Default)]
pub(crate) struct NodeIds {
    next: u32,
    free: Vec<u32>,
    quarantined: Vec<u32>,
}

impl NodeIds {
    pub(crate) fn alloc(&mut self) -> NodeId {
        NodeId(self.free.pop().unwrap_or_else(|| {
            let id = self.next;
            self.next += 1;
            id
        }))
    }

    pub(crate) fn free(&mut self, id: NodeId) {
        self.quarantined.push(id.0);
    }
}

pub(crate) fn promote_free_ids(mut ids: ResMut<NodeIds>) {
    let NodeIds {
        free, quarantined, ..
    } = &mut *ids;
    free.append(quarantined);
}

/// Opcodes of the DOM command stream. Layouts are documented on the
/// [`DomCommandBuffer`] emitters and must match `interpret` in
/// `js/registry.js` exactly. Strings are `(offset, len)` pairs into the
/// flush's arena, measured in UTF-16 code units (what `String.substring`
/// expects on the JS side).
mod op {
    /// `(NOP, skip)` — skip the next `skip` words. Written over cancelled
    /// creates, never emitted directly.
    pub const NOP: u32 = 0;
    /// `(op, id, str tag, ent_lo, ent_hi)`
    pub const CREATE_ELEMENT: u32 = 1;
    /// `(op, id, str ns, str tag, ent_lo, ent_hi)`
    pub const CREATE_ELEMENT_NS: u32 = 2;
    /// `(op, id, str text, ent_lo, ent_hi)`
    pub const CREATE_TEXT: u32 = 3;
    /// `(op, id, str text)`
    pub const SET_TEXT: u32 = 4;
    /// `(op, id, str name, str value)`
    pub const SET_ATTRIBUTE: u32 = 5;
    /// `(op, id, str name)`
    pub const REMOVE_ATTRIBUTE: u32 = 6;
    /// `(op, id, str class)`
    pub const ADD_CLASS: u32 = 7;
    /// `(op, id, str name, str value)`
    pub const SET_PROPERTY_STR: u32 = 8;
    /// `(op, id, str name, value)`
    pub const SET_PROPERTY_BOOL: u32 = 9;
    /// `(op, id, str html)`
    pub const SET_INNER_HTML: u32 = 10;
    /// `(op, parent, child)`
    pub const APPEND: u32 = 11;
    /// `(op, parent, child, anchor)`
    pub const INSERT_BEFORE: u32 = 12;
}

/// Entity bits packed into two u32 words for the op stream; the all-ones
/// sentinel means "skip reverse-lookup registration" (generation overflow —
/// see `node_lookup::encode`).
const ENTITY_SKIP: (u32, u32) = (u32::MAX, u32::MAX);

/// The buffered DOM write stream for one `PostUpdate`. Write systems in
/// `[DomSystems::Insert, DomSystems::Attach]` push opcodes instead of making
/// per-call Wasm↔JS crossings; `flush_commands` (`DomSystems::Flush`) hands
/// the whole stream to `interpret` in a single crossing.
///
/// The buffer is only non-empty inside that window. Everything that runs at
/// arbitrary command-apply points (despawn hooks, removal observers) stays
/// on direct calls — using the handles' non-panicking `fetch` accessor,
/// since teardown can race a still-buffered create or an already-removed
/// node. Handle *derefs* must happen outside the window (Update, event
/// dispatch, post-Flush systems); consumer systems and observers that need
/// to write from inside it query [`NodeId`] and emit through this buffer
/// instead.
#[derive(Resource, Default)]
pub struct DomCommandBuffer {
    ops: Vec<u32>,
    arena: String,
    /// UTF-16 length of `arena` — string operands index the decoded JS
    /// string, not the utf-8 bytes.
    arena_units: usize,
    /// Creates emitted this window, by id → op offset, so a node despawned
    /// before its creating flush can be cancelled in place.
    pending_creates: HashMap<u32, usize>,
}

impl DomCommandBuffer {
    fn push_str(&mut self, s: &str) {
        self.ops.push(self.arena_units as u32);
        let units = s.encode_utf16().count();
        self.ops.push(units as u32);
        self.arena.push_str(s);
        self.arena_units += units;
    }

    fn entity_bits(entity: Entity) -> (u32, u32) {
        match super::html::encode_entity(entity) {
            Some(value) => {
                let bits = value as u64;
                (bits as u32, (bits >> 32) as u32)
            }
            None => ENTITY_SKIP,
        }
    }

    fn create(&mut self, opcode: u32, id: NodeId) {
        self.pending_creates.insert(id.0, self.ops.len());
        self.ops.push(opcode);
        self.ops.push(id.0);
    }

    pub(crate) fn create_element(&mut self, id: NodeId, tag: &str, entity: Entity) {
        self.create(op::CREATE_ELEMENT, id);
        self.push_str(tag);
        let (lo, hi) = Self::entity_bits(entity);
        self.ops.extend([lo, hi]);
    }

    pub(crate) fn create_element_ns(&mut self, id: NodeId, ns: &str, tag: &str, entity: Entity) {
        self.create(op::CREATE_ELEMENT_NS, id);
        self.push_str(ns);
        self.push_str(tag);
        let (lo, hi) = Self::entity_bits(entity);
        self.ops.extend([lo, hi]);
    }

    pub(crate) fn create_text(&mut self, id: NodeId, text: &str, entity: Entity) {
        self.create(op::CREATE_TEXT, id);
        self.push_str(text);
        let (lo, hi) = Self::entity_bits(entity);
        self.ops.extend([lo, hi]);
    }

    pub(crate) fn set_text(&mut self, id: NodeId, text: &str) {
        self.ops.extend([op::SET_TEXT, id.0]);
        self.push_str(text);
    }

    /// Buffer a `setAttribute` for a managed node.
    ///
    /// Public for consumer systems that run inside the `PostUpdate` write
    /// window (between [`DomSystems::Insert`] and [`DomSystems::Flush`]) —
    /// including `Start`/insert observers that fire when handle components
    /// land. Handles must not be dereferenced there (the node may not be
    /// flushed yet); querying [`NodeId`] and emitting through the buffer is
    /// the supported way to write from that window.
    ///
    /// [`DomSystems::Insert`]: crate::dom::DomSystems::Insert
    /// [`DomSystems::Flush`]: crate::dom::DomSystems::Flush
    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) {
        self.ops.extend([op::SET_ATTRIBUTE, id.0]);
        self.push_str(name);
        self.push_str(value);
    }

    pub(crate) fn remove_attribute(&mut self, id: NodeId, name: &str) {
        self.ops.extend([op::REMOVE_ATTRIBUTE, id.0]);
        self.push_str(name);
    }

    pub(crate) fn add_class(&mut self, id: NodeId, class: &str) {
        self.ops.extend([op::ADD_CLASS, id.0]);
        self.push_str(class);
    }

    pub(crate) fn set_property_str(&mut self, id: NodeId, name: &str, value: &str) {
        self.ops.extend([op::SET_PROPERTY_STR, id.0]);
        self.push_str(name);
        self.push_str(value);
    }

    pub(crate) fn set_property_bool(&mut self, id: NodeId, name: &str, value: bool) {
        self.ops.extend([op::SET_PROPERTY_BOOL, id.0]);
        self.push_str(name);
        self.ops.push(value as u32);
    }

    pub(crate) fn set_inner_html(&mut self, id: NodeId, html: &str) {
        self.ops.extend([op::SET_INNER_HTML, id.0]);
        self.push_str(html);
    }

    pub(crate) fn append(&mut self, parent: NodeId, child: NodeId) {
        self.ops.extend([op::APPEND, parent.0, child.0]);
    }

    pub(crate) fn insert_before(&mut self, parent: NodeId, child: NodeId, anchor: NodeId) {
        self.ops.extend([op::INSERT_BEFORE, parent.0, child.0, anchor.0]);
    }

    /// A node spawned and despawned inside the same dirty window never
    /// reaches the DOM: NOP-patch its create in place (later ops on the id
    /// no-op through the interpreter's missing-node guard). Returns whether
    /// a pending create was cancelled.
    pub(crate) fn cancel_pending_create(&mut self, id: NodeId) -> bool {
        let Some(index) = self.pending_creates.remove(&id.0) else {
            return false;
        };
        let words = match self.ops[index] {
            op::CREATE_ELEMENT | op::CREATE_TEXT => 6,
            op::CREATE_ELEMENT_NS => 8,
            other => unreachable!("pending create pointing at opcode {other}"),
        };
        self.ops[index] = op::NOP;
        self.ops[index + 1] = words - 2;
        true
    }

    pub(crate) fn flush(&mut self) -> core::result::Result<(), JsValue> {
        if self.ops.is_empty() {
            return Ok(());
        }
        let result = interpret(&self.ops, &self.arena);
        self.ops.clear();
        self.arena.clear();
        self.arena_units = 0;
        self.pending_creates.clear();
        result
    }
}

pub(crate) fn flush_commands(mut buffer: ResMut<DomCommandBuffer>) -> Result {
    use crate::js_err::JsErr;
    buffer.flush().js_err()
}

#[wasm_bindgen(module = "/js/registry.js")]
extern "C" {
    /// Put `node` into the registry under `id`. When `entity_bits` is given,
    /// the node → entity reverse mapping (event hit-testing) is registered
    /// alongside.
    pub(crate) fn adopt(id: u32, node: &web_sys::Node, entity_bits: Option<f64>);

    /// The registry slot for `id`; `undefined` if empty.
    pub(crate) fn get_node(id: u32) -> JsValue;

    /// Detach the node from the DOM and drop it from the registry.
    pub(crate) fn remove_node(id: u32);

    /// Packed entity bits registered for exactly this node, if any.
    pub(crate) fn lookup_entity(node: &web_sys::Node) -> Option<f64>;

    /// Packed entity bits of the nearest registered ancestor (starting at
    /// `node` itself), walking `parentNode` entirely JS-side.
    pub(crate) fn nearest_entity(node: &web_sys::Node) -> Option<f64>;

    /// Number of live registry entries — a leak guard for tests.
    #[doc(hidden)]
    pub fn live_nodes() -> u32;

    /// Execute a whole [`DomCommandBuffer`] stream in one crossing. Throws
    /// (with the failing op index) if an op fails; individual ops targeting
    /// missing registry slots are skipped, which is what makes NOP-patched
    /// creates and despawn races benign.
    #[wasm_bindgen(catch)]
    pub(crate) fn interpret(ops: &[u32], strings: &str) -> core::result::Result<(), JsValue>;
}
