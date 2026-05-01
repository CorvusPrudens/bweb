use bevy_ecs::{prelude::*, system::SystemParam};
use js_sys::WeakMap;
use send_wrapper::SendWrapper;
use wasm_bindgen::{JsCast, JsValue};

const GEN_BITS: u32 = 21;
const GEN_MAX: u32 = (1 << GEN_BITS) - 1;

#[derive(Resource)]
pub(crate) struct NodeEntityMap(SendWrapper<WeakMap>);

impl core::default::Default for NodeEntityMap {
    fn default() -> Self {
        Self(SendWrapper::new(WeakMap::new()))
    }
}

#[inline]
fn encode(entity: Entity) -> Option<f64> {
    let gen_bits = entity.generation().to_bits();
    if gen_bits > GEN_MAX {
        log::warn!(
            "entity {entity:?} generation {gen_bits} exceeds 21-bit packing; \
             skipping reverse-lookup registration",
        );
        return None;
    }
    Some(entity.to_bits() as f64)
}

#[inline]
fn decode(value: f64) -> Option<Entity> {
    Entity::try_from_bits(value as u64)
}

pub(super) fn register(map: &NodeEntityMap, node: &web_sys::Node, entity: Entity) {
    let Some(value) = encode(entity) else { return };
    map.0.set(node, &JsValue::from_f64(value));
}

#[derive(SystemParam)]
pub struct NodeLookup<'w> {
    map: Res<'w, NodeEntityMap>,
}

impl NodeLookup<'_> {
    pub fn get(&self, node: &web_sys::Node) -> Option<Entity> {
        decode(self.map.0.get(node).as_f64()?)
    }

    pub fn event_target(&self, value: impl AsRef<web_sys::Event>) -> Option<Entity> {
        let target = value.as_ref().target()?;
        let node = target.dyn_into::<web_sys::Node>().ok()?;
        self.nearest_entity(&node)
    }

    /// Walks this node's ancestor chain until an ECS-created
    /// node is found, if any.
    pub fn nearest_entity(&self, node: &web_sys::Node) -> Option<Entity> {
        let mut node = Some(node.clone());

        while let Some(next) = node {
            match self.map.0.get(&next).as_f64() {
                Some(entity) => return decode(entity),
                None => {
                    node = next.parent_node();
                }
            }
        }

        None
    }
}
