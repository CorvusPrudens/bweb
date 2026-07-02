use crate::dom::registry;
use bevy_ecs::{prelude::*, system::SystemParam};
use core::marker::PhantomData;
use wasm_bindgen::JsCast;

const GEN_BITS: u32 = 21;
const GEN_MAX: u32 = (1 << GEN_BITS) - 1;

/// Packs an entity into the f64 the JS-side node → entity map stores
/// (registered at adoption time, `registry::adopt`).
#[inline]
pub(crate) fn encode(entity: Entity) -> Option<f64> {
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

/// Resolves live DOM nodes (typically event targets) back to the entities
/// whose handles created them. The mapping itself lives JS-side next to the
/// node registry.
#[derive(SystemParam)]
pub struct NodeLookup<'w> {
    _marker: PhantomData<&'w ()>,
}

impl NodeLookup<'_> {
    pub fn get(&self, node: &web_sys::Node) -> Option<Entity> {
        decode(registry::lookup_entity(node)?)
    }

    pub fn event_target(&self, value: impl AsRef<web_sys::Event>) -> Option<Entity> {
        let target = value.as_ref().target()?;
        let node = target.dyn_into::<web_sys::Node>().ok()?;
        self.nearest_entity(&node)
    }

    /// Walks this node's ancestor chain until an ECS-created
    /// node is found, if any.
    pub fn nearest_entity(&self, node: &web_sys::Node) -> Option<Entity> {
        decode(registry::nearest_entity(node)?)
    }
}
