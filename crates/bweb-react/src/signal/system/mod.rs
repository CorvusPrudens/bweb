use bevy_app::prelude::*;
use bevy_ecs::{
    prelude::*,
    system::{SystemChangeTick, SystemId},
};
use std::{
    any::Any,
    sync::{Arc, Mutex},
};

use super::reactive_observer::SubscriberSet;
use crate::{ReactSchedule, ReactScheduleSystems, Reactions};

mod derived;
mod memo;

pub use derived::DerivedSystem;
pub use memo::MemoSystem;

pub struct SystemPlugin;

impl Plugin for SystemPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SignalEvaluations>().add_systems(
            ReactSchedule,
            (
                SignalState::collect_memos,
                SignalState::evaluate_memos,
                SignalState::detect_changes,
                SignalState::evaluate,
            )
                .chain()
                .in_set(ReactScheduleSystems::EvaluateSignals),
        );
    }
}

pub struct SignalState {
    system: SignalSystemId,
    pub outputs: SignalOutputs,
}

#[derive(Component)]
pub struct SignalStateContainer(pub Option<SignalState>);

impl SignalStateContainer {
    pub fn new(system: SignalSystemId, outputs: SignalOutputs) -> Self {
        Self(Some(SignalState::new(system, outputs)))
    }

    pub fn insert(&mut self, func: Inserter, entity: Entity) {
        self.0
            .as_mut()
            .expect("signal state should be available")
            .outputs
            .insert(func, entity);
    }

    pub fn remove(&mut self, entity: Entity) {
        self.0
            .as_mut()
            .expect("signal state should be available")
            .outputs
            .remove(entity);
    }
}

impl SignalState {
    pub fn new(system: SignalSystemId, outputs: SignalOutputs) -> Self {
        Self { system, outputs }
    }

    fn detect_changes(
        signals: Query<(Entity, &SubscriberSet, &SignalStateContainer)>,
        world: &World,
        tick: SystemChangeTick,
        evals: Res<SignalEvaluations>,
    ) {
        let mut evals = evals.0.lock().unwrap();
        for (entity, set, state) in &signals {
            if set.has_changed(world, tick.last_run(), tick.this_run()) {
                evals.push(Evaluation {
                    entity,
                    inputs_changed: true,
                });
            } else if !state.0.as_ref().unwrap().outputs.new_inserters.is_empty() {
                evals.push(Evaluation {
                    entity,
                    inputs_changed: false,
                });
            }
        }
    }

    fn evaluate(world: &mut World) -> Result {
        world.resource_scope::<SignalEvaluations, _>(|world, evals| -> Result {
            let mut reactions = 0;
            for Evaluation {
                entity: eval,
                inputs_changed,
            } in evals.0.lock().unwrap().drain(..)
            {
                if evaluate_signal(world, eval, inputs_changed) {
                    reactions += 1;
                }
            }

            world.resource_mut::<Reactions>().count += reactions;

            Ok(())
        })
    }

    fn collect_memos(
        signals: Query<Entity, (With<SignalStateContainer>, Without<SubscriberSet>)>,
        evals: Res<SignalEvaluations>,
    ) {
        let mut evals = evals.0.lock().unwrap();
        evals.extend(signals.iter().map(|entity| Evaluation {
            entity,
            inputs_changed: false,
        }));
    }

    fn evaluate_memos(world: &mut World) -> Result {
        world.resource_scope::<SignalEvaluations, _>(|world, evals| -> Result {
            let mut reactions = 0;
            for Evaluation { entity: eval, .. } in evals.0.lock().unwrap().drain(..) {
                if evaluate_memo(world, eval) {
                    reactions += 1;
                }
            }
            world.resource_mut::<Reactions>().count += reactions;

            Ok(())
        })
    }
}

pub(crate) fn evaluate_signal(world: &mut World, signal: Entity, inputs_changed: bool) -> bool {
    let Some(mut state) = world
        .get_mut::<SignalStateContainer>(signal)
        .and_then(|mut s| s.0.take())
    else {
        return false;
    };
    state.outputs.inputs_changed = inputs_changed;

    let set = world.get::<SubscriberSet>(signal).cloned().unwrap();

    set.clear();
    let result = super::reactive_observer::SignalObserver::observe(&set, || {
        world.run_system_with(state.system, &mut state.outputs)
    });

    if let Some(mut container) = world.get_mut::<SignalStateContainer>(signal) {
        container.0 = Some(state);
    }

    if let Err(e) = result {
        log::error!("Failed to run signal system: {e}");
    }

    true
}

pub(crate) fn evaluate_memo(world: &mut World, memo: Entity) -> bool {
    let mut reacted = false;

    let Some(mut state) = world
        .get_mut::<SignalStateContainer>(memo)
        .and_then(|mut s| s.0.take())
    else {
        return false;
    };

    state.outputs.outputs_changed = false;

    let result = world.run_system_with(state.system, &mut state.outputs);

    if state.outputs.outputs_changed {
        reacted = true;
    }

    if let Some(mut container) = world.get_mut::<SignalStateContainer>(memo) {
        container.0 = Some(state);
    }

    if let Err(e) = result {
        log::error!("Failed to run signal system: {e}");
    }

    reacted
}

pub type SignalSystemId = SystemId<InMut<'static, SignalOutputs>>;

#[derive(Clone)]
pub struct SignalOutputs {
    inputs_changed: bool,
    outputs_changed: bool,
    inserters: Vec<(Inserter, Entity)>,
    new_inserters: Vec<(Inserter, Entity)>,
}

impl SignalOutputs {
    pub fn new() -> Self {
        Self {
            inputs_changed: true,
            outputs_changed: false,
            inserters: Vec::new(),
            new_inserters: Vec::new(),
        }
    }

    fn insert(&mut self, func: Inserter, entity: Entity) {
        self.new_inserters.push((func, entity));
    }

    fn remove(&mut self, entity: Entity) {
        self.new_inserters.retain(|(_, e)| *e != entity);
        self.inserters.retain(|(_, e)| *e != entity);
    }

    fn process_all_inserters(&mut self, mut func: impl FnMut(&Inserter, Entity)) {
        for (inserter, entity) in self.inserters.iter() {
            func(inserter, *entity);
        }
        self.process_new_inserters(func);
    }

    fn process_new_inserters(&mut self, mut func: impl FnMut(&Inserter, Entity)) {
        for (inserter, entity) in self.new_inserters.drain(..) {
            func(&inserter, entity);
            self.inserters.push((inserter, entity));
        }
    }
}

pub type InserterFn = Arc<dyn Fn(&dyn Any, EntityCommands) + Send + Sync>;

#[derive(Clone)]
pub enum Inserter {
    Ptr(fn(&dyn Any, EntityCommands)),
    Arc(InserterFn),
}

impl Inserter {
    pub fn call(&self, any: &dyn Any, entity: EntityCommands) {
        match self {
            Self::Arc(arc) => {
                arc(any, entity);
            }
            Self::Ptr(ptr) => {
                ptr(any, entity);
            }
        }
    }
}

#[derive(Resource, Default)]
struct SignalEvaluations(Mutex<Vec<Evaluation>>);

struct Evaluation {
    entity: Entity,
    inputs_changed: bool,
}
