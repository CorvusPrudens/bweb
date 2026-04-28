use bevy_app::{App, Plugin};
use bevy_ecs::{
    component::ComponentId, lifecycle::HookContext, prelude::*, system::BoxedSystem,
    world::DeferredWorld,
};
use bevy_platform::collections::HashMap;
use std::{
    marker::PhantomData,
    sync::{Arc, RwLock},
};

pub struct CleanupPlugin;

impl Plugin for CleanupPlugin {
    fn build(&self, app: &mut App) {
        app.register_reactive_cleanup::<Children>();
    }
}

#[derive(Default, Resource)]
pub(crate) struct CleanupRegistry {
    registry: Arc<RwLock<CleanupMap>>,
}

impl CleanupRegistry {
    pub fn from_world(world: &World) -> Self {
        let registry = world.resource::<Self>().registry.clone();
        Self { registry }
    }

    pub fn perform_cleanup(&self, components: &[ComponentId], entity: &mut EntityWorldMut) {
        let registry = self.registry.read().unwrap();
        for cleanup in components.iter().filter_map(|c| registry.get(c).copied()) {
            cleanup(entity);
        }
    }
}

type CleanupMap = HashMap<ComponentId, fn(&mut EntityWorldMut)>;

pub trait ReactiveCleanup {
    fn reactive_cleanup(entity: &mut EntityWorldMut);
}

impl<C: RelationshipTarget> ReactiveCleanup for C {
    fn reactive_cleanup(entity: &mut EntityWorldMut) {
        if C::LINKED_SPAWN {
            entity.despawn_related::<C>();
        }
    }
}

pub trait RegisterReactiveCleanup {
    fn register_reactive_cleanup<C: Component + ReactiveCleanup>(&mut self) -> &mut Self;
}

impl RegisterReactiveCleanup for App {
    fn register_reactive_cleanup<C: Component + ReactiveCleanup>(&mut self) -> &mut Self {
        self.world_mut().register_reactive_cleanup::<C>();
        self
    }
}

impl RegisterReactiveCleanup for World {
    fn register_reactive_cleanup<C: Component + ReactiveCleanup>(&mut self) -> &mut Self {
        let id = self.register_component::<C>();
        let registry = self
            .get_resource_or_init::<CleanupRegistry>()
            .registry
            .clone();
        registry.write().unwrap().insert(id, C::reactive_cleanup);
        self
    }
}

pub struct ReactiveCleanupCommand<B: Bundle>(PhantomData<B>);

impl<B: Bundle> ReactiveCleanupCommand<B> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

impl<B: Bundle> EntityCommand for ReactiveCleanupCommand<B> {
    fn apply(self, mut entity: EntityWorldMut) {
        let components = entity.world().components();
        let component_collection: Vec<_> = B::get_component_ids(components).flatten().collect();

        let registry = CleanupRegistry::from_world(entity.world());
        registry.perform_cleanup(&component_collection, &mut entity);
    }
}

pub trait ReactiveCleanupExt {
    fn reactive_cleanup<B: Bundle>(&mut self) -> &mut Self;
}

impl ReactiveCleanupExt for EntityCommands<'_> {
    fn reactive_cleanup<B: Bundle>(&mut self) -> &mut Self {
        self.queue(ReactiveCleanupCommand::<B>::new())
    }
}

impl ReactiveCleanupExt for EntityWorldMut<'_> {
    fn reactive_cleanup<B: Bundle>(&mut self) -> &mut Self {
        let components = self.world().components();
        let component_collection: Vec<_> = B::get_component_ids(components).flatten().collect();

        let registry = crate::cleanup::CleanupRegistry::from_world(self.world());
        registry.perform_cleanup(&component_collection, self);
        self
    }
}

#[derive(Component)]
#[component(on_replace = Self::replace)]
pub struct Cleanup(Option<BoxedSystem<In<Entity>>>);

impl Cleanup {
    pub fn new<S, M>(system: S) -> Self
    where
        S: IntoSystem<In<Entity>, (), M>,
    {
        Self(Some(Box::new(IntoSystem::into_system(system))))
    }

    fn replace(mut world: DeferredWorld, context: HookContext) {
        let Some(mut system) = world
            .get_mut::<Self>(context.entity)
            .and_then(|mut c| c.0.take())
        else {
            return;
        };

        world.commands().queue(move |world: &mut World| -> Result {
            system.initialize(world);
            system
                .run(context.entity, world)
                .map_err(|e| format!("failed to execute cleanup: {e}"))?;

            Ok(())
        });
    }
}
