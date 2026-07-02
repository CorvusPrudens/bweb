use super::registry::{self, DomCommandBuffer, NodeId, NodeIds};
use super::{DomChildren, DomParent, DomStartupSystems, DomSystems};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::{lifecycle::HookContext, prelude::*, world::DeferredWorld};
use bevy_query_observer::{AddStopObserver, Stop};
use std::borrow::Cow;
use std::sync::OnceLock;
use wasm_bindgen::JsCast;

pub mod elements;
mod inner_html;
mod node_lookup;
pub mod svg;

pub use inner_html::InnerHtml;
pub use node_lookup::NodeLookup;
pub(crate) use node_lookup::encode as encode_entity;

pub(super) struct HtmlPlugin;

impl Plugin for HtmlPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((svg::SvgPlugin, InnerHtml::plugin))
            .add_systems(
                PreStartup,
                initialize_window.in_set(DomStartupSystems::Window),
            )
            .add_systems(
                PostUpdate,
                (
                    update_text,
                    inject_element,
                    inject_input_element,
                    inject_select_element,
                    inject_text_area_element,
                    inject_text,
                )
                    .chain()
                    .in_set(DomSystems::Insert),
            )
            .add_stop_observer(remove_text);
    }
}

#[doc(hidden)]
pub use send_wrapper::SendWrapper;

#[macro_export]
macro_rules! web_wrapper {
    ($ty:ident) => {
        #[derive(Component, Clone)]
        #[cfg_attr(feature = "debug", derive(Debug))]
        pub struct $ty($crate::dom::html::SendWrapper<web_sys::$ty>);

        impl $ty {
            pub fn new(value: web_sys::$ty) -> Self {
                Self($crate::dom::html::SendWrapper::new(value))
            }
        }

        impl core::ops::Deref for $ty {
            type Target = web_sys::$ty;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl AsRef<web_sys::$ty> for $ty {
            fn as_ref(&self) -> &web_sys::$ty {
                &self.0
            }
        }
    };
    // A lazily-resolved handle to a managed node in bweb's JS-side registry:
    // only the first deref crosses the Wasm↔JS boundary (fetching from the
    // registry), after which the live handle is cached. Internal to bweb —
    // the registry plumbing it expands to is crate-private.
    (lazy $ty:ident) => {
        #[derive(Component, Clone)]
        #[cfg_attr(feature = "debug", derive(Debug))]
        pub struct $ty {
            /// `None` for foreign values wrapped via [`Self::new`], which
            /// live outside the registry and are always warm.
            id: Option<$crate::dom::registry::NodeId>,
            cell: ::std::sync::OnceLock<$crate::dom::html::SendWrapper<web_sys::$ty>>,
        }

        impl $ty {
            /// Wrap a foreign value that does not live in the node registry.
            pub fn new(value: web_sys::$ty) -> Self {
                let cell = ::std::sync::OnceLock::new();
                let _ = cell.set($crate::dom::html::SendWrapper::new(value));
                Self { id: None, cell }
            }

            /// A cold handle, resolved from the registry on first deref.
            #[allow(dead_code)]
            pub(crate) fn lazy(id: $crate::dom::registry::NodeId) -> Self {
                Self {
                    id: Some(id),
                    cell: ::std::sync::OnceLock::new(),
                }
            }

            /// A warm handle for a node already adopted into the registry.
            #[allow(dead_code)]
            pub(crate) fn adopted(
                id: $crate::dom::registry::NodeId,
                value: web_sys::$ty,
            ) -> Self {
                let cell = ::std::sync::OnceLock::new();
                let _ = cell.set($crate::dom::html::SendWrapper::new(value));
                Self { id: Some(id), cell }
            }

            /// This handle's registry id; `None` for foreign values.
            pub fn node_id(&self) -> Option<$crate::dom::registry::NodeId> {
                self.id
            }

            /// The live handle, if it has already been fetched.
            pub fn try_get(&self) -> Option<&web_sys::$ty> {
                self.cell.get().map(|value| &**value)
            }

            /// Like deref, but `None` (instead of a panic) when the node is
            /// not in the registry — already removed, or not yet created.
            ///
            /// This is what teardown paths (removal observers, despawn
            /// cleanup) must use: they can run while the target's create is
            /// still buffered or after the node was already dropped, and in
            /// both cases there is nothing to clean up.
            pub fn fetch(&self) -> Option<&web_sys::$ty> {
                if let Some(value) = self.cell.get() {
                    return Some(&**value);
                }
                let node = $crate::dom::registry::get_node(self.id?.0);
                if node.is_undefined() {
                    return None;
                }
                Some(&**self.cell.get_or_init(|| {
                    $crate::dom::html::SendWrapper::new(
                        <web_sys::$ty as ::wasm_bindgen::JsCast>::unchecked_from_js(node),
                    )
                }))
            }
        }

        impl core::ops::Deref for $ty {
            type Target = web_sys::$ty;

            fn deref(&self) -> &Self::Target {
                self.cell.get_or_init(|| {
                    let id = self.id.expect("foreign handles are constructed warm");
                    let node = $crate::dom::registry::get_node(id.0);
                    assert!(
                        !node.is_undefined(),
                        concat!(
                            "bweb: `",
                            stringify!($ty),
                            "` handle dereferenced for {:?}, which is not in the \
                             node registry (already removed, or not yet created)"
                        ),
                        id,
                    );
                    $crate::dom::html::SendWrapper::new(
                        <web_sys::$ty as ::wasm_bindgen::JsCast>::unchecked_from_js(node),
                    )
                })
            }
        }

        impl AsRef<web_sys::$ty> for $ty {
            fn as_ref(&self) -> &web_sys::$ty {
                self
            }
        }
    };
}

web_wrapper!(Window);
web_wrapper!(Document);
web_wrapper!(HtmlDocument);
web_wrapper!(Navigator);

fn initialize_window(mut ids: ResMut<NodeIds>, mut commands: Commands) -> Result {
    let window = web_sys::window().ok_or("browser window should be available")?;
    let window_target: &web_sys::EventTarget = &window;

    commands.spawn((
        Navigator(SendWrapper::new(window.navigator())),
        EventTarget::new(window_target.clone()),
        Window(SendWrapper::new(window.clone())),
    ));

    let document = window
        .document()
        .ok_or("browser document should be available")?;
    let document_node: &web_sys::Node = &document;
    let document_html = document.unchecked_ref::<web_sys::HtmlDocument>();

    let document_id = ids.alloc();
    let document_entity = commands
        .spawn((
            document_id,
            Node::adopted(document_id, document_node.clone()),
            Document(SendWrapper::new(document.clone())),
            HtmlDocument(SendWrapper::new(document_html.clone())),
        ))
        .id();
    registry::adopt(
        document_id.0,
        document_node,
        node_lookup::encode(document_entity),
    );

    let html = document
        .document_element()
        .ok_or("document HTML should be available")?;
    let html_node: web_sys::Node = html.clone().unchecked_into();
    let html_id = ids.alloc();
    let html_entity = commands
        .spawn((
            html_id,
            elements::Html,
            HtmlElement::adopted(html_id, html.clone().unchecked_into()),
            Element::adopted(html_id, html),
            Node::adopted(html_id, html_node.clone()),
        ))
        .id();
    registry::adopt(html_id.0, &html_node, node_lookup::encode(html_entity));
    let html = html_entity;

    let head = document.head().ok_or("document head should be available")?;
    let head_node: web_sys::Node = head.clone().unchecked_into();
    let head_id = ids.alloc();
    let head_entity = commands
        .spawn((
            head_id,
            ChildOf(html),
            elements::Head,
            HtmlElement::adopted(head_id, head.clone().unchecked_into()),
            Element::adopted(head_id, head.unchecked_into()),
            Node::adopted(head_id, head_node.clone()),
        ))
        .id();
    registry::adopt(head_id.0, &head_node, node_lookup::encode(head_entity));

    let body = document.body().ok_or("document body should be available")?;
    let body_node: web_sys::Node = body.clone().unchecked_into();
    let body_id = ids.alloc();
    let body_entity = commands
        .spawn((
            body_id,
            ChildOf(html),
            elements::Body,
            HtmlElement::adopted(body_id, body.clone()),
            Element::adopted(body_id, body.unchecked_into()),
            Node::adopted(body_id, body_node.clone()),
            crate::relative_mouse::RelativeMouse::default(),
        ))
        .id();
    registry::adopt(body_id.0, &body_node, node_lookup::encode(body_entity));

    Ok(())
}

web_wrapper!(lazy HtmlElement);
web_wrapper!(lazy HtmlInputElement);
web_wrapper!(lazy HtmlSelectElement);
web_wrapper!(lazy HtmlTextAreaElement);
web_wrapper!(lazy Element);
web_wrapper!(lazy EventTarget);
web_wrapper!(lazy SvgElement);

#[derive(Component)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[component(on_replace = Self::on_remove_hook, on_insert = Self::on_insert_hook)]
#[require(DomChildren, DomParent)]
pub struct Node {
    id: NodeId,
    cell: OnceLock<SendWrapper<web_sys::Node>>,
}

impl Node {
    /// A cold handle, resolved from the registry on first deref.
    pub(crate) fn lazy(id: NodeId) -> Self {
        Self {
            id,
            cell: OnceLock::new(),
        }
    }

    /// A warm handle for a node already adopted into the registry.
    pub(crate) fn adopted(id: NodeId, value: web_sys::Node) -> Self {
        let cell = OnceLock::new();
        let _ = cell.set(SendWrapper::new(value));
        Self { id, cell }
    }

    /// This handle's registry id.
    pub fn node_id(&self) -> NodeId {
        self.id
    }

    /// The live handle, if it has already been fetched.
    pub fn try_get(&self) -> Option<&web_sys::Node> {
        self.cell.get().map(|value| &**value)
    }

    /// Like deref, but `None` (instead of a panic) when the node is not in
    /// the registry — already removed, or not yet created. Teardown paths
    /// must use this; see the lazy `web_wrapper!` arm.
    pub fn fetch(&self) -> Option<&web_sys::Node> {
        if let Some(value) = self.cell.get() {
            return Some(&**value);
        }
        let node = registry::get_node(self.id.0);
        if node.is_undefined() {
            return None;
        }
        Some(&**self
            .cell
            .get_or_init(|| SendWrapper::new(node.unchecked_into())))
    }
}

impl core::ops::Deref for Node {
    type Target = web_sys::Node;

    fn deref(&self) -> &Self::Target {
        self.cell.get_or_init(|| {
            let node = registry::get_node(self.id.0);
            assert!(
                !node.is_undefined(),
                "bweb: `Node` handle dereferenced for {:?}, which is not in the \
                 node registry (already removed, or not yet created)",
                self.id,
            );
            SendWrapper::new(node.unchecked_into())
        })
    }
}

impl AsRef<web_sys::Node> for Node {
    fn as_ref(&self) -> &web_sys::Node {
        self
    }
}

impl Node {
    fn on_remove_hook(mut world: DeferredWorld, context: HookContext) {
        let id = world.get::<Node>(context.entity).unwrap().id;
        // Spawned and despawned inside the same tick's dirty window: the
        // node was never created, so cancel its buffered create instead.
        if !world
            .resource_mut::<DomCommandBuffer>()
            .cancel_pending_create(id)
        {
            registry::remove_node(id.0);
        }
        world.resource_mut::<NodeIds>().free(id);

        // Keep the DOM mirror exact: this node just left the DOM.
        let parent = world
            .get::<DomParent>(context.entity)
            .and_then(|dom_parent| dom_parent.0);
        if let Some(parent) = parent {
            if let Some(mut siblings) = world.get_mut::<DomChildren>(parent) {
                siblings.0.retain(|&sibling| sibling != context.entity);
            }
            if let Some(mut dom_parent) = world.get_mut::<DomParent>(context.entity) {
                dom_parent.0 = None;
            }
        }
    }

    fn on_insert_hook(mut world: DeferredWorld, context: HookContext) {
        let id = world.get::<Node>(context.entity).unwrap().id;

        world
            .commands()
            .entity(context.entity)
            .insert(EventTarget::lazy(id));
    }
}

/// An HTML element inserter.
#[derive(Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
#[cfg_attr(feature = "reflect", reflect(Component))]
#[component(on_replace = Self::on_replace_hook)]
pub struct HtmlElementName(pub &'static str);

impl HtmlElementName {
    fn on_replace_hook(mut world: DeferredWorld, context: HookContext) {
        if let Ok(mut entity) = world.commands().get_entity(context.entity) {
            entity.try_remove::<(
                NodeId,
                Node,
                EventTarget,
                HtmlElement,
                Element,
                HtmlInputElement,
                HtmlTextAreaElement,
                HtmlSelectElement,
            )>();
        }
    }
}

fn inject_element(
    elements: Query<(Entity, &HtmlElementName), Without<Node>>,
    mut ids: ResMut<NodeIds>,
    mut buffer: ResMut<DomCommandBuffer>,
    mut commands: Commands,
) {
    for (entity, element) in &elements {
        let id = ids.alloc();
        buffer.create_element(id, element.0, entity);

        commands.entity(entity).insert((
            id,
            Element::lazy(id),
            HtmlElement::lazy(id),
            Node::lazy(id),
        ));
    }
}

fn inject_input_element(
    elements: Query<
        (Entity, &NodeId),
        (With<elements::Input>, With<Node>, Without<HtmlInputElement>),
    >,
    mut commands: Commands,
) {
    for (entity, &id) in &elements {
        commands.entity(entity).insert(HtmlInputElement::lazy(id));
    }
}

fn inject_select_element(
    elements: Query<
        (Entity, &NodeId),
        (
            With<elements::Select>,
            With<Node>,
            Without<HtmlSelectElement>,
        ),
    >,
    mut commands: Commands,
) {
    for (entity, &id) in &elements {
        commands.entity(entity).insert(HtmlSelectElement::lazy(id));
    }
}

fn inject_text_area_element(
    elements: Query<
        (Entity, &NodeId),
        (
            With<elements::TextArea>,
            With<Node>,
            Without<HtmlTextAreaElement>,
        ),
    >,
    mut commands: Commands,
) {
    for (entity, &id) in &elements {
        commands
            .entity(entity)
            .insert(HtmlTextAreaElement::lazy(id));
    }
}

#[derive(Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
#[cfg_attr(feature = "reflect", reflect(Component))]
// #[component(on_remove = Self::remove)]
pub struct Text(Cow<'static, str>);

impl core::ops::Deref for Text {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Text {
    pub fn new(text: impl Into<Cow<'static, str>>) -> Self {
        Self(text.into())
    }

    // fn remove(mut world: DeferredWorld, context: HookContext) {
    //     if let Ok(mut entity) = world.commands().get_entity(context.entity) {
    //         entity.try_remove::<Node>();
    //     }
    // }
}

fn inject_text(
    texts: Query<(Entity, &Text), Without<Node>>,
    mut ids: ResMut<NodeIds>,
    mut buffer: ResMut<DomCommandBuffer>,
    mut commands: Commands,
) {
    for (entity, text) in &texts {
        let id = ids.alloc();
        // The create op carries the initial content, so creation-tick text
        // costs no extra op — this system only handles later edits.
        buffer.create_text(id, &text.0, entity);
        commands.entity(entity).insert((id, Node::lazy(id)));
    }
}

fn update_text(
    texts: Query<(&Text, &NodeId), Changed<Text>>,
    mut buffer: ResMut<DomCommandBuffer>,
) {
    for (text, id) in &texts {
        buffer.set_text(*id, &text.0);
    }
}

fn remove_text(
    text: Stop<(Entity, &Node, &ChildOf), With<Text>>,
    parent: Query<&Element>,
    mut mirror_children: Query<&mut DomChildren>,
    mut mirror_parents: Query<&mut DomParent>,
) -> Result {
    let (entity, text, child_of) = text.into_inner();
    // `fetch`, not deref: teardown can run while either node's create is
    // still buffered or after it was dropped — nothing to detach then.
    let Some((parent, text)) = parent
        .get(child_of.0)
        .ok()
        .and_then(Element::fetch)
        .zip(text.fetch())
    else {
        return Ok(());
    };

    parent.remove_child(text).js_err()?;

    // Keep the DOM mirror exact: the text node just left the DOM.
    if let Ok(mut siblings) = mirror_children.get_mut(child_of.0) {
        siblings.0.retain(|&sibling| sibling != entity);
    }
    if let Ok(mut dom_parent) = mirror_parents.get_mut(entity) {
        dom_parent.0 = None;
    }

    Ok(())
}
