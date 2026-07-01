//! The signal constructors: the [`SignalExt`] extension trait on `Commands` and
//! its shared node-registration helper.

use bevy_ecs::{
    prelude::*,
    query::{QueryData, QueryFilter},
};
use bevy_query_observer::{
    QueryObserver, Start,
    observer::{QueryObserverAccess, TriggerQueryObserver},
};
use std::sync::{Arc, Mutex, RwLock};

use super::error::SignalResult;
use super::gc::SignalGc;
use super::graph::{
    ChangedNodes, ClosureEval, NodeStatus, PendingDirty, Polled, SignalClosure, SignalSystem,
    Sources, Subscribers, evaluate_node,
};
use super::handle::{
    DerivedSignal, ObserverShared, ObserverSignal, SignalInner, WatchTarget, build_observer,
};
use super::reactive_context::ReactiveContext;

/// Signal constructors, added to `Commands`.
pub trait SignalExt {
    /// Creates an input signal driven by a query observer. Fires whenever the
    /// watched entity gains or changes the queried component(s). Watch a specific
    /// entity with [`ObserverSignal::watch_entity`] /
    /// [`ObserverSignal::watch_bundle`]; the default watches every match.
    ///
    /// [`ObserverSignal::watch_entity`]: super::ObserverSignal::watch_entity
    /// [`ObserverSignal::watch_bundle`]: super::ObserverSignal::watch_bundle
    fn signal<S, D, F, M, O>(&mut self, system: S) -> ObserverSignal<O>
    where
        S: IntoSystem<Start<'static, 'static, D, F>, O, M> + Send + Sync + 'static,
        D: QueryData + QueryObserverAccess + 'static,
        F: QueryFilter + QueryObserverAccess + 'static,
        O: Send + Sync + 'static;

    /// Creates a derived signal recomputed whenever a signal it reads changes.
    ///
    /// Takes a plain closure returning `Result<O, SignalError>` — most derives
    /// only read other signals via `.get()?`, so this avoids the overhead of a
    /// registered system. For a derive that needs `Query`/`Res`, a
    /// `derive_system`-style method can be layered on later.
    fn derive<F, O>(&mut self, closure: F) -> DerivedSignal<O>
    where
        F: FnMut() -> SignalResult<O> + Send + Sync + 'static,
        O: Clone + Send + Sync + 'static;

    /// Like [`derive`](Self::derive), but propagates to subscribers only when the
    /// output actually changes (`PartialEq`). A recompute that yields an equal
    /// value prunes the subtree below it.
    fn memo<F, O>(&mut self, closure: F) -> DerivedSignal<O>
    where
        F: FnMut() -> SignalResult<O> + Send + Sync + 'static,
        O: PartialEq + Clone + Send + Sync + 'static;

    /// Like [`memo`](Self::memo), but re-evaluated on *every* flush pass rather
    /// than only when a source changes — use it to track world state that emits
    /// no lifecycle event for a query observer. Propagation stays value-gated, so
    /// a poll whose value is stable prunes its subgraph.
    fn poll<S, O, M>(&mut self, system: S) -> DerivedSignal<O>
    where
        S: IntoSystem<(), SignalResult<O>, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: PartialEq + Clone + Send + Sync + 'static;
}

impl SignalExt for Commands<'_, '_> {
    fn signal<S, D, F, M, O>(&mut self, system: S) -> ObserverSignal<O>
    where
        S: IntoSystem<Start<'static, 'static, D, F>, O, M> + Send + Sync + 'static,
        D: QueryData + QueryObserverAccess + 'static,
        F: QueryFilter + QueryObserverAccess + 'static,
        O: Send + Sync + 'static,
    {
        let node = self
            .spawn((
                Subscribers::default(),
                Sources::default(),
                NodeStatus::Dirty,
            ))
            .id();
        let shared = Arc::new(ObserverShared {
            value: RwLock::new(None),
            builder: Mutex::new(None),
            watch: Mutex::new(WatchTarget::Global),
        });
        self.entity(node).insert(SignalGc::new(&shared, 2));

        let piped = system.pipe({
            let shared = shared.clone();
            move |output: In<O>, mut pending: ResMut<PendingDirty>| {
                *shared.value.write().unwrap() = Some(output.0);
                pending.0.push(node);
            }
        });

        // Deferred: build the query observer once the watched entity is known.
        // The observer lives on the signal node; `None` watches every match.
        *shared.builder.lock().unwrap() = Some(Box::new(move |world: &mut World, watched| {
            let mut observer = QueryObserver::start(piped);
            if let Some(entity) = watched {
                observer = observer.with_entity(entity);
            }
            observer.insert_into(node, world);
            // Seed the current value by triggering once for the watched entity.
            // (A global watcher has no single entity to seed from.)
            if let Some(entity) = watched {
                world.trigger_query_observer(node, entity);
            }
        }));

        // Finalize `Global`/`Entity` now; `Bundle` waits for `WatchBundle`.
        self.queue({
            let shared = shared.clone();
            move |world: &mut World| match &*shared.watch.lock().unwrap() {
                WatchTarget::Bundle => {}
                WatchTarget::Global => build_observer(&shared, world, None),
                WatchTarget::Entity(entity) => build_observer(&shared, world, Some(*entity)),
            }
        });

        ObserverSignal {
            inner: SignalInner { entity: node },
            shared,
        }
    }

    fn derive<F, O>(&mut self, mut closure: F) -> DerivedSignal<O>
    where
        F: FnMut() -> SignalResult<O> + Send + Sync + 'static,
        O: Clone + Send + Sync + 'static,
    {
        let value = Arc::new(RwLock::new(None));
        let eval: ClosureEval = Box::new({
            let value = value.clone();
            move || {
                let (result, sources) = ReactiveContext::collect(&mut closure);
                // Ok(v) -> Some(v); Err(NotReady) -> None (propagates downstream).
                // A plain derive counts every recompute as a change.
                *value.write().unwrap() = result.ok();
                (true, sources)
            }
        });
        register_closure(self, value, eval)
    }

    fn memo<F, O>(&mut self, mut closure: F) -> DerivedSignal<O>
    where
        F: FnMut() -> SignalResult<O> + Send + Sync + 'static,
        O: PartialEq + Clone + Send + Sync + 'static,
    {
        let value = Arc::new(RwLock::new(None));
        let eval: ClosureEval = Box::new({
            let value = value.clone();
            move || {
                let (result, sources) = ReactiveContext::collect(&mut closure);
                // Propagate only when the output actually moves.
                let new = result.ok();
                let mut cell = value.write().unwrap();
                let changed = *cell != new;
                if changed {
                    *cell = new;
                }
                (changed, sources)
            }
        });
        register_closure(self, value, eval)
    }

    fn poll<S, O, M>(&mut self, system: S) -> DerivedSignal<O>
    where
        S: IntoSystem<(), SignalResult<O>, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: PartialEq + Clone + Send + Sync + 'static,
    {
        let node = self.spawn_empty().id();
        let value = Arc::new(RwLock::new(None));

        let piped = system.pipe({
            let value = value.clone();
            move |out: In<SignalResult<O>>, mut changed: ResMut<ChangedNodes>| {
                // Value-gated (memo) sink: propagate only when the output moves.
                let new = out.0.ok();
                let mut cell = value.write().unwrap();
                if *cell != new {
                    *cell = new;
                    changed.0.insert(node);
                }
            }
        });

        let handle = register_system_node(self, node, value, piped);
        self.entity(node).insert(Polled);
        handle
    }
}

/// Node setup for the closure constructors ([`SignalExt::derive`] /
/// [`SignalExt::memo`]): store the type-erased evaluator, insert the graph
/// components, and queue the initial evaluation.
fn register_closure<O>(
    commands: &mut Commands,
    value: Arc<RwLock<Option<O>>>,
    eval: ClosureEval,
) -> DerivedSignal<O>
where
    O: Send + Sync + 'static,
{
    let node = commands
        .spawn((
            Subscribers::default(),
            Sources::default(),
            NodeStatus::Dirty,
            SignalClosure(eval),
            SignalGc::new(&value, 2),
        ))
        .id();

    // Evaluate once so the initial edges and value are populated; the flush
    // drives subsequent re-evaluations.
    commands.queue(move |world: &mut World| {
        evaluate_node(world, node);
        if let Some(mut status) = world.get_mut::<NodeStatus>(node) {
            *status = NodeStatus::Clean;
        }
    });

    DerivedSignal {
        inner: SignalInner { entity: node },
        value,
    }
}

/// Node setup for system-backed nodes ([`SignalExt::poll`], and future
/// `*_system` constructors): register the `piped` system, insert the graph
/// components, and queue the initial evaluation. The `piped` system writes the
/// value cell and marks [`ChangedNodes`] when it should propagate.
fn register_system_node<PS, PM, O>(
    commands: &mut Commands,
    node: Entity,
    value: Arc<RwLock<Option<O>>>,
    piped: PS,
) -> DerivedSignal<O>
where
    PS: IntoSystem<(), (), PM> + Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    // The system is unregistered when the node is garbage-collected (`despawn_node`
    // via `gc_pass`).
    let system = commands.register_system(piped);
    commands.entity(node).insert((
        Subscribers::default(),
        Sources::default(),
        NodeStatus::Dirty,
        SignalSystem(system),
        SignalGc::new(&value, 2),
    ));

    commands.queue(move |world: &mut World| {
        evaluate_node(world, node);
        if let Some(mut status) = world.get_mut::<NodeStatus>(node) {
            *status = NodeStatus::Clean;
        }
    });

    DerivedSignal {
        inner: SignalInner { entity: node },
        value,
    }
}
