use bevy_ecs::{
    lifecycle::HookContext,
    prelude::*,
    query::{QueryData, QueryFilter},
    world::DeferredWorld,
};
use bevy_query_observer::{
    QueryObserver, SpawnQueryObserver, Start,
    observer::{QueryObserverAccess, TriggerQueryObserver},
};
use core::marker::PhantomData;

use crate::cleanup::ReactiveCleanupExt;

#[derive(Component)]
#[component(on_insert = Self::insert, on_replace = Self::replace)]
pub struct SignalObserver<O: Bundle> {
    observer: Option<Box<dyn FnOnce(&mut World, SignalObserverTargets) -> Entity + Send + Sync>>,
    observer_entity: Option<Entity>,
    input: Option<Entity>,
    output: PhantomData<fn() -> O>,
}

struct SignalObserverTargets {
    input: Entity,
    output: Entity,
}

impl<O: Bundle> SignalObserver<O> {
    pub fn new<S, D, F, M>(observer: S) -> Self
    where
        S: IntoSystem<Start<'static, 'static, D, F>, O, M> + Send + Sync + 'static,
        D: QueryData + QueryObserverAccess + 'static,
        F: QueryFilter + QueryObserverAccess + 'static,
    {
        Self {
            observer: Some(Box::new(
                move |world: &mut World, targets: SignalObserverTargets| {
                    let output = targets.output;
                    let piped = observer.pipe(move |bundle: In<O>, mut commands: Commands| {
                        commands
                            .entity(output)
                            .reactive_cleanup::<O>()
                            .try_insert(bundle.0);
                    });

                    let observer = QueryObserver::start(piped).with_entity(targets.input);

                    world.spawn_query_observer(observer)
                },
            )),
            observer_entity: None,
            input: None,
            output: PhantomData,
        }
    }

    pub fn target(self, entity: Entity) -> Self {
        Self {
            input: Some(entity),
            ..self
        }
    }

    fn insert(mut world: DeferredWorld, context: HookContext) {
        world.commands().queue(move |world: &mut World| -> Result {
            let mut entity = world.get_entity_mut(context.entity)?;
            let mut entity = entity.get_mut::<Self>().ok_or_else(|| {
                format!("Expected `{}` component", core::any::type_name::<Self>())
            })?;

            let input = entity.input.unwrap_or(context.entity);
            let observer = entity
                .observer
                .take()
                .ok_or("Observer system should be present")?;
            let observer_entity = observer(
                world,
                SignalObserverTargets {
                    input,
                    output: context.entity,
                },
            );

            world
                .entity_mut(context.entity)
                .get_mut::<Self>()
                .unwrap()
                .observer_entity = Some(observer_entity);

            world.trigger_query_observer(observer_entity, input);

            Ok(())
        });
    }

    fn replace(mut world: DeferredWorld, context: HookContext) {
        if let Some(entity) = world
            .get::<Self>(context.entity)
            .expect("`SignalObserver` component should be present")
            .observer_entity
        {
            if let Ok(mut entity) = world.commands().get_entity(entity) {
                entity.despawn();
            }
        }
    }
}
