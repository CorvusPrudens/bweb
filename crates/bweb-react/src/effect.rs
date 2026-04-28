use super::{ReactSchedule, ReactScheduleSystems, Reactions};
use bevy_app::prelude::*;
use bevy_ecs::{
    lifecycle::HookContext,
    prelude::*,
    system::{SystemChangeTick, SystemId},
    world::DeferredWorld,
};
use std::sync::Mutex;

use crate::signal::{
    SignalMarker,
    reactive_observer::{self, SubscriberSet},
};

pub(crate) struct EffectPlugin;

impl Plugin for EffectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EffectEvaluations>().add_systems(
            ReactSchedule,
            (detect_changes, evaluate)
                .chain()
                .in_set(ReactScheduleSystems::EvaluateEffects),
        );
    }
}

#[derive(Component, Clone)]
#[component(on_replace = Self::on_replace_hook)]
pub struct Effect {
    pub(crate) entity: Entity,
}

impl Effect {
    fn on_replace_hook(mut world: DeferredWorld, context: HookContext) {
        let Some(data) = world.get::<Self>(context.entity) else {
            return;
        };
        let entity = data.entity;
        world.commands().entity(entity).despawn();
    }
}

#[derive(Component, Clone, Copy)]
#[component(on_replace = Self::on_replace_hook)]
struct EffectState {
    system: SystemId,
}

impl EffectState {
    fn on_replace_hook(mut world: DeferredWorld, context: HookContext) {
        let Some(data) = world.get::<Self>(context.entity) else {
            return;
        };

        let system = data.system;
        world.commands().unregister_system(system);
    }
}

impl Effect {
    pub(crate) fn new<S, M>(system: S, mut commands: Commands) -> Self
    where
        S: IntoSystem<(), (), M> + Send + Sync + 'static,
        M: 'static,
    {
        let system = commands.register_system(system);
        commands.entity(system.entity()).insert(SignalMarker);
        let set = SubscriberSet::new();
        let entity = commands
            .spawn((EffectState { system }, set.clone(), SignalMarker))
            .id();

        Self { entity }
    }
}

fn detect_changes(
    signals: Query<(Entity, Ref<SubscriberSet>), With<EffectState>>,
    world: &World,
    tick: SystemChangeTick,
    evals: Res<EffectEvaluations>,
) {
    let mut evals = evals.0.lock().unwrap();
    for (entity, set) in &signals {
        if set.is_added() || set.has_changed(world, tick.last_run(), tick.this_run()) {
            evals.push(entity);
        }
    }
}

fn evaluate(world: &mut World) -> Result {
    world.resource_scope::<EffectEvaluations, _>(|world, evals| -> Result {
        let mut reactions = 0;
        for eval in evals.0.lock().unwrap().drain(..) {
            let Some(&EffectState { system }) = world.get::<EffectState>(eval) else {
                continue;
            };
            let Some(set) = world.get::<SubscriberSet>(eval).cloned() else {
                continue;
            };

            set.clear();
            reactive_observer::SignalObserver::observe(&set, || world.run_system(system))?;

            reactions += 1;
        }

        world.resource_mut::<Reactions>().count += reactions;

        Ok(())
    })
}

#[derive(Resource, Default)]
struct EffectEvaluations(Mutex<Vec<Entity>>);
