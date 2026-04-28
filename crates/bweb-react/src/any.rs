use bevy_ecs::{
    bundle::Bundle,
    component::Component,
    lifecycle::HookContext,
    world::{DeferredWorld, EntityWorldMut, World},
};

use crate::cleanup::ReactiveCleanupExt;

pub trait IntoAnyBundle {
    fn into_any(self) -> AnyBundle;
}

impl<B: Bundle> IntoAnyBundle for B {
    fn into_any(self) -> AnyBundle {
        AnyBundle {
            inserter: Some(Box::new(move |mut world| {
                world.insert(self);
            })),
            cleanup: |mut entity| {
                entity.reactive_cleanup::<B>();
                entity.remove::<B>();
            },
        }
    }
}

#[derive(Component)]
#[component(on_insert = Self::on_insert_hook, on_replace = Self::on_replace_hook)]
pub struct AnyBundle {
    inserter: Option<Box<dyn FnOnce(EntityWorldMut) + Send + Sync>>,
    cleanup: AnyBundleCleanup,
}

type AnyBundleCleanup = fn(EntityWorldMut);

impl AnyBundle {
    fn on_insert_hook(mut world: DeferredWorld, context: HookContext) {
        let inserter = world
            .get_mut::<Self>(context.entity)
            .and_then(|mut c| c.inserter.take())
            .expect("`AnyBundle` hook should access self");

        world
            .commands()
            .queue(move |world: &mut World| -> bevy_ecs::error::Result {
                let entity = world.get_entity_mut(context.entity)?;
                inserter(entity);
                Ok(())
            });
    }

    fn on_replace_hook(mut world: DeferredWorld, context: HookContext) {
        let cleanup = world
            .get::<Self>(context.entity)
            .map(|c| c.cleanup)
            .expect("`AnyBundle` hook should access self");

        world
            .commands()
            .queue(move |world: &mut World| -> bevy_ecs::error::Result {
                // The component may be despawned already
                if let Ok(entity) = world.get_entity_mut(context.entity) {
                    cleanup(entity);
                }
                Ok(())
            });
    }
}
