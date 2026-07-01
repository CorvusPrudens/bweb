use bevy_app::prelude::*;
use bevy_ecs::{
    component::{Mutable, StorageType},
    lifecycle::{ComponentHook, HookContext},
    prelude::*,
    query::{QueryData, QueryFilter},
    system::SystemId,
    world::DeferredWorld,
};
use bevy_platform::collections::{HashMap, HashSet};
use bevy_query_observer::{QueryObserver, Start, observer::QueryObserverAccess};
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use crate::cleanup::ReactiveCleanupExt;

mod error;
mod reactive_context;

pub use error::{SignalError, SignalReadGuard, SignalResult};
use reactive_context::ReactiveContext;

/// Wires up the signal2 reactive runtime.
pub struct Signal2Plugin;

impl Plugin for Signal2Plugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingDirty>()
            .init_resource::<ChangedNodes>()
            .add_systems(PostUpdate, flush);
    }
}

/// Input nodes whose value changed since the last flush.
///
/// Fed by the query observers backing [`ObserverSignal`]s; drained by the flush,
/// which marks each source's subscriber subgraph (see [`mark_from_source`]).
#[derive(Resource, Default)]
struct PendingDirty(Vec<Entity>);

/// Nodes whose value changed during the current flush pass.
///
/// Seeded with the inputs that fired, then extended as derived nodes recompute.
/// A `Check` node recomputes only if one of its sources is in this set. Cleared
/// between fixpoint passes.
///
/// Named `Changed*` to avoid colliding with [`bevy_ecs::prelude::Changed`].
#[derive(Resource, Default)]
struct ChangedNodes(HashSet<Entity>);

/// Scheduler status of a reactive node.
///
/// - `Clean`: up to date.
/// - `Check`: a transitive source *might* have changed; sources must be settled
///   before deciding whether to recompute.
/// - `Dirty`: a direct source changed; the node must recompute.
#[derive(Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum NodeStatus {
    Clean,
    Check,
    Dirty,
}

/// Forward edges: the subscriber entities that read this node (publisher).
///
/// Maintained in place during the flush; the reverse of [`Sources`].
#[derive(Component, Default)]
pub struct Subscribers(Vec<Entity>);

/// Backward edges: the source entities this node reads.
///
/// Rewritten by [`rewire_edges`] after each evaluation from the set collected by
/// [`ReactiveContext`]; the reverse of [`Subscribers`].
#[derive(Component, Default)]
pub struct Sources(Vec<Entity>);

/// How a derived node recomputes. Input/source nodes (driven by a query
/// observer) have no `SignalSystem`.
#[derive(Component, Clone, Copy)]
pub struct SignalSystem(SystemId);

#[derive(Clone)]
struct SignalInner {
    entity: Entity,
}

pub struct ObserverSignal<O> {
    inner: SignalInner,
    target: Option<Entity>,
    input: Option<Entity>,
    value: Arc<RwLock<Option<O>>>,
}

// Hand-written (not derived) so cloning a handle never requires `O: Clone` — the
// only `O` lives behind an `Arc`.
impl<O> Clone for ObserverSignal<O> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            target: self.target,
            input: self.input,
            value: self.value.clone(),
        }
    }
}

pub struct DerivedSignal<O> {
    inner: SignalInner,
    value: Arc<RwLock<Option<O>>>,
}

impl<O> Clone for DerivedSignal<O> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            value: self.value.clone(),
        }
    }
}

/// Fallible read access shared by every signal handle.
///
/// Reads register the signal as a source of whatever node is currently
/// evaluating (via [`ReactiveContext`]), and error with [`SignalError::NotReady`]
/// until the signal has produced a value.
pub trait SignalRead: Clone + Send + Sync + 'static {
    type Value;

    /// Reads the current value by reference.
    fn read(&self) -> SignalResult<SignalReadGuard<'_, Self::Value>>;

    /// Reads and clones the current value.
    fn get(&self) -> SignalResult<Self::Value>
    where
        Self::Value: Clone,
    {
        Ok(self.read()?.clone())
    }

    /// Reactively maps this signal's value **by reference** into an insertable
    /// bundle. The result is a [`MappedSignal`] component: dropped onto an entity
    /// it (re)inserts `f`'s output whenever this signal changes.
    fn map<F, O2>(&self, f: F) -> MappedSignal<O2>
    where
        Self: Sized,
        F: Fn(&Self::Value) -> O2 + Send + Sync + 'static,
        O2: Bundle + Send + Sync + 'static,
    {
        let source = self.clone();
        let f = Arc::new(f);
        ReactiveInsert {
            spawn: Arc::new(move |commands: &mut Commands, host: Entity| {
                spawn_map_sink(commands, host, source.clone(), f.clone())
            }),
            _marker: PhantomData,
        }
    }
}

/// Shared read implementation: register the source, then guard the value.
fn read_value<O>(
    entity: Entity,
    value: &Arc<RwLock<Option<O>>>,
) -> SignalResult<SignalReadGuard<'_, O>> {
    // Register before the readiness check so a not-yet-ready read still subscribes.
    ReactiveContext::register(entity);
    let guard = value.read().unwrap();
    if guard.is_none() {
        return Err(SignalError::NotReady);
    }
    Ok(SignalReadGuard(guard))
}

impl<O: Send + Sync + 'static> SignalRead for DerivedSignal<O> {
    type Value = O;

    fn read(&self) -> SignalResult<SignalReadGuard<'_, O>> {
        read_value(self.inner.entity, &self.value)
    }
}

impl<O: Send + Sync + 'static> SignalRead for ObserverSignal<O> {
    type Value = O;

    fn read(&self) -> SignalResult<SignalReadGuard<'_, O>> {
        read_value(self.inner.entity, &self.value)
    }
}

pub trait Signal {
    fn signal<S, D, F, M, O>(&mut self, system: S) -> ObserverSignal<O>
    where
        S: IntoSystem<Start<'static, 'static, D, F>, O, M> + Send + Sync + 'static,
        D: QueryData + QueryObserverAccess + 'static,
        F: QueryFilter + QueryObserverAccess + 'static,
        O: Send + Sync + 'static;

    fn derive<S, O, M>(&mut self, system: S) -> DerivedSignal<O>
    where
        S: IntoSystem<(), SignalResult<O>, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: Clone + Send + Sync + 'static;
}

impl Signal for Commands<'_, '_> {
    fn signal<S, D, F, M, O>(&mut self, system: S) -> ObserverSignal<O>
    where
        S: IntoSystem<Start<'static, 'static, D, F>, O, M> + Send + Sync + 'static,
        D: QueryData + QueryObserverAccess + 'static,
        F: QueryFilter + QueryObserverAccess + 'static,
        O: Send + Sync + 'static,
    {
        let observer_entity = self
            .spawn((
                Subscribers::default(),
                Sources::default(),
                NodeStatus::Dirty,
            ))
            .id();
        let inner = SignalInner {
            entity: observer_entity,
        };
        let value = Arc::new(RwLock::new(None));

        let piped = system.pipe({
            let value = value.clone();
            move |output: In<O>, mut pending: ResMut<PendingDirty>| {
                *value.write().unwrap() = Some(output.0);
                pending.0.push(observer_entity);
            }
        });
        let observer = QueryObserver::start(piped);

        self.queue(move |world: &mut World| {
            observer.insert_into(observer_entity, world);
        });

        ObserverSignal {
            inner,
            input: None,
            target: None,
            value,
        }
    }

    fn derive<S, O, M>(&mut self, system: S) -> DerivedSignal<O>
    where
        S: IntoSystem<(), SignalResult<O>, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: Clone + Send + Sync + 'static,
    {
        let node = self.spawn_empty().id();
        let inner = SignalInner { entity: node };
        let value = Arc::new(RwLock::new(None));

        let piped = system.pipe({
            let value = value.clone();
            move |out: In<SignalResult<O>>| {
                // Ok(v) -> Some(v); Err(NotReady) -> None (propagates downstream).
                *value.write().unwrap() = out.0.ok();
            }
        });
        // TODO: unregister the system on cleanup.
        let system = self.register_system(piped);

        self.entity(node).insert((
            Subscribers::default(),
            Sources::default(),
            NodeStatus::Dirty,
            SignalSystem(system),
        ));

        // Evaluate once so the initial edges and value are populated; the flush
        // drives subsequent re-evaluations.
        self.queue(move |world: &mut World| {
            evaluate_node(world, node);
            if let Some(mut status) = world.get_mut::<NodeStatus>(node) {
                *status = NodeStatus::Clean;
            }
        });

        DerivedSignal { inner, value }
    }
}

/// Maximum fixpoint passes per flush before bailing out. A well-formed graph
/// settles in one pass; extra passes only occur when a sink's side effect (e.g.
/// a component insertion) trips an input observer, or a mid-flush rewire surfaces
/// a newly-changed source out of order.
const REACTION_LIMIT: usize = 16;

/// Propagates all pending input changes through the reactive graph.
///
/// Each pass: mark the active subgraph from the drained inputs, settle it in
/// topological order, then loop if a sink enqueued new work — up to
/// [`REACTION_LIMIT`].
fn flush(world: &mut World) {
    for _ in 0..REACTION_LIMIT {
        let (inputs, active) = drain_and_mark(world);
        if active.is_empty() {
            break;
        }

        // The inputs that fired have, by definition, changed.
        {
            let mut changed = world.resource_mut::<ChangedNodes>();
            changed.0.extend(inputs);
        }

        settle_active(world, active);

        world.resource_mut::<ChangedNodes>().0.clear();
        if world.resource::<PendingDirty>().0.is_empty() {
            break;
        }
    }
}

/// Drains [`PendingDirty`] and marks the subscriber subgraph of each changed
/// input. Returns `(fired inputs, active nodes)`; the active set is every node
/// whose status was raised out of `Clean` this pass, each listed once.
fn drain_and_mark(world: &mut World) -> (Vec<Entity>, Vec<Entity>) {
    let inputs = core::mem::take(&mut world.resource_mut::<PendingDirty>().0);
    let mut active = Vec::new();
    for &source in &inputs {
        mark_from_source(world, source, &mut active);
    }
    (inputs, active)
}

/// Marks the dependency subgraph rooted at a changed `source`.
///
/// Direct subscribers become [`NodeStatus::Dirty`]; transitive subscribers
/// become [`NodeStatus::Check`]. A node is only walked when its status actually
/// rises, and `Check` is pushed downward only when a node leaves `Clean` — a
/// `Check`→`Dirty` upgrade already propagated `Check` to its descendants, so the
/// traversal short-circuits and stays O(newly marked subgraph). Each node is
/// pushed to `active` exactly once, when it first leaves `Clean`.
fn mark_from_source(world: &mut World, source: Entity, active: &mut Vec<Entity>) {
    let mut stack: Vec<(Entity, NodeStatus)> = world
        .get::<Subscribers>(source)
        .map(|subs| subs.0.iter().map(|&e| (e, NodeStatus::Dirty)).collect())
        .unwrap_or_default();

    while let Some((node, level)) = stack.pop() {
        let Some(current) = world.get::<NodeStatus>(node).copied() else {
            continue;
        };
        let raised = current.max(level);
        if raised == current {
            // Already at or above this level; its descendants are handled.
            continue;
        }
        *world.get_mut::<NodeStatus>(node).unwrap() = raised;

        if current == NodeStatus::Clean {
            active.push(node);
            if let Some(subs) = world.get::<Subscribers>(node) {
                for &sub in &subs.0 {
                    stack.push((sub, NodeStatus::Check));
                }
            }
        }
    }
}

/// Settles the `active` set in topological order (Kahn over the active
/// sub-DAG), so every node runs only after all of its active sources have.
///
/// If some nodes remain unsettled after the ordered pass — a dependency cycle,
/// or edges rewired mid-flush — they are force-settled best-effort so no node is
/// left non-`Clean`.
fn settle_active(world: &mut World, active: Vec<Entity>) {
    let active_set: HashSet<Entity> = active.iter().copied().collect();

    // In-degree = number of a node's sources that are themselves active. Sources
    // outside the active set are already settled, so their values are final.
    let mut in_degree: HashMap<Entity, u32> = HashMap::with_capacity(active.len());
    for &node in &active {
        let deg = world
            .get::<Sources>(node)
            .map(|s| s.0.iter().filter(|src| active_set.contains(*src)).count() as u32)
            .unwrap_or(0);
        in_degree.insert(node, deg);
    }

    let mut ready: Vec<Entity> = active
        .iter()
        .copied()
        .filter(|node| in_degree[node] == 0)
        .collect();

    let mut settled = 0usize;
    while let Some(node) = ready.pop() {
        settle_node(world, node);
        settled += 1;

        let subscribers: Vec<Entity> = world
            .get::<Subscribers>(node)
            .map(|s| s.0.clone())
            .unwrap_or_default();
        for sub in subscribers {
            if let Some(degree) = in_degree.get_mut(&sub) {
                *degree = degree.saturating_sub(1);
                if *degree == 0 {
                    ready.push(sub);
                }
            }
        }
    }

    if settled < active.len() {
        log::warn!(
            "signal2: {} node(s) unsettled (cycle or mid-flush rewire); forcing settle",
            active.len() - settled
        );
        for &node in &active {
            if world.get::<NodeStatus>(node).copied() != Some(NodeStatus::Clean) {
                settle_node(world, node);
            }
        }
    }
}

/// Settles a single node: recomputes it if it must, records whether its value
/// changed, then resets it to `Clean`.
///
/// A `Dirty` node always recomputes. A `Check` node recomputes only if one of
/// its sources landed in [`ChangedNodes`] this pass.
///
/// TODO: value-equality pruning (memo) currently treats every recompute as a
/// change. Once a `PartialEq` memo constructor lands, the output sink should
/// populate [`ChangedNodes`] conditionally instead.
fn settle_node(world: &mut World, node: Entity) {
    let status = world
        .get::<NodeStatus>(node)
        .copied()
        .unwrap_or(NodeStatus::Clean);

    let should_run = match status {
        NodeStatus::Dirty => true,
        NodeStatus::Check => {
            let sources: Vec<Entity> = world
                .get::<Sources>(node)
                .map(|s| s.0.clone())
                .unwrap_or_default();
            let changed = &world.resource::<ChangedNodes>().0;
            sources.iter().any(|src| changed.contains(src))
        }
        NodeStatus::Clean => false,
    };

    if should_run {
        evaluate_node(world, node);
        world.resource_mut::<ChangedNodes>().0.insert(node);
    }

    if let Some(mut status) = world.get_mut::<NodeStatus>(node) {
        *status = NodeStatus::Clean;
    }
}

/// Runs a derived node's system while collecting its source reads, then
/// reconciles its edges. A no-op for input nodes (no [`SignalSystem`]).
fn evaluate_node(world: &mut World, node: Entity) {
    let Some(SignalSystem(system)) = world.get::<SignalSystem>(node).copied() else {
        return;
    };
    let (result, sources) = ReactiveContext::collect(|| world.run_system(system));
    if let Err(e) = result {
        log::error!("Failed to run signal system: {e}");
    }
    rewire_edges(world, node, &sources);
}

/// Reconciles `node`'s dependency edges after an evaluation.
///
/// `new_sources` is the (possibly duplicated) set of source entities read during
/// the run. This deduplicates them, then updates both directions in place:
/// removes `node` from the [`Subscribers`] of sources it no longer reads, adds it
/// to those it newly reads, and stores the deduped set as `node`'s [`Sources`].
fn rewire_edges(world: &mut World, node: Entity, new_sources: &[Entity]) {
    let new_set: HashSet<Entity> = new_sources.iter().copied().collect();
    let old: Vec<Entity> = world
        .get::<Sources>(node)
        .map(|s| s.0.clone())
        .unwrap_or_default();
    let old_set: HashSet<Entity> = old.iter().copied().collect();

    // Sources no longer read: unsubscribe this node.
    for removed in old_set.difference(&new_set) {
        if let Some(mut subs) = world.get_mut::<Subscribers>(*removed) {
            subs.0.retain(|e| *e != node);
        }
    }

    // Newly read sources: subscribe this node.
    for added in new_set.difference(&old_set) {
        if let Some(mut subs) = world.get_mut::<Subscribers>(*added) {
            if !subs.0.contains(&node) {
                subs.0.push(node);
            }
        }
    }

    if let Some(mut sources) = world.get_mut::<Sources>(node) {
        sources.0 = new_set.into_iter().collect();
    }
}

// ---------------------------------------------------------------------------
// Reactive Bundle insertion
// ---------------------------------------------------------------------------

/// Spawns a graph node whose system runs for side effects (not a readable
/// value). Unlike [`Signal::derive`] the system may use `Commands` (it is not a
/// `ReadOnlySystem`) and there is no value cell. Used for reactive sinks —
/// component insertion, mapping, `option`.
fn spawn_effect<Sys, M>(commands: &mut Commands, system: Sys) -> Entity
where
    Sys: IntoSystem<(), (), M> + Send + Sync + 'static,
{
    let node = commands.spawn_empty().id();
    let system = commands.register_system(system);
    commands.entity(node).insert((
        Subscribers::default(),
        Sources::default(),
        NodeStatus::Dirty,
        SignalSystem(system),
    ));
    commands.queue(move |world: &mut World| {
        evaluate_node(world, node);
        if let Some(mut status) = world.get_mut::<NodeStatus>(node) {
            *status = NodeStatus::Clean;
        }
    });
    node
}

/// Tears down a graph node: unsubscribe from all sources, unregister its system,
/// and despawn it. Safe to call on a sink whose host is gone.
fn despawn_node(world: &mut World, node: Entity) {
    rewire_edges(world, node, &[]);
    if let Some(SignalSystem(system)) = world.get::<SignalSystem>(node).copied() {
        let _ = world.unregister_system(system);
    }
    world.despawn(node);
}

/// Spawns a sink that reads `source` by reference, maps it through `f`, and
/// reactively (re)inserts the resulting `O2` bundle into `host`, cleaning up the
/// prior value. Skips insertion while the source is [`SignalError::NotReady`],
/// leaving whatever was last inserted.
fn spawn_map_sink<S, F, O2>(commands: &mut Commands, host: Entity, source: S, f: Arc<F>) -> Entity
where
    S: SignalRead,
    F: Fn(&S::Value) -> O2 + Send + Sync + 'static,
    O2: Bundle + Send + Sync + 'static,
{
    spawn_effect(commands, move |mut commands: Commands| {
        match source.read() {
            Ok(guard) => {
                let bundle = f(&guard);
                commands
                    .entity(host)
                    .reactive_cleanup::<O2>()
                    .try_insert(bundle);
            }
            Err(SignalError::NotReady) => {}
        }
    })
}

type SpawnSink = Arc<dyn Fn(&mut Commands, Entity) -> Entity + Send + Sync>;

/// A type-erased reactive insertion. Dropped onto a host entity it spawns a sink
/// that (re)inserts a `K` bundle derived from some source signal. Produced by
/// [`SignalRead::map`] ([`MappedSignal`]) and [`SignalOption::option`]
/// ([`OptionSignal`]).
pub struct ReactiveInsert<K> {
    spawn: SpawnSink,
    _marker: PhantomData<fn() -> K>,
}

/// A signal value reactively mapped into an insertable bundle. See
/// [`SignalRead::map`].
pub type MappedSignal<O> = ReactiveInsert<O>;

/// An `Option`-valued signal reactively inserted as a bundle, removing the
/// bundle when the value is `None`. See [`SignalOption::option`].
pub type OptionSignal<O> = ReactiveInsert<O>;

impl<K> Clone for ReactiveInsert<K> {
    fn clone(&self) -> Self {
        Self {
            spawn: self.spawn.clone(),
            _marker: PhantomData,
        }
    }
}

impl<K: Send + Sync + 'static> Component for ReactiveInsert<K> {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;

    fn on_insert() -> Option<ComponentHook> {
        fn hook<K: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
            let spawn = world
                .get::<ReactiveInsert<K>>(ctx.entity)
                .unwrap()
                .spawn
                .clone();
            let host = ctx.entity;
            let mut commands = world.commands();
            let sink = spawn(&mut commands, host);
            bind_sink::<K>(&mut commands, host, sink);
        }
        Some(hook::<K>)
    }

    fn on_replace() -> Option<ComponentHook> {
        Some(unbind_sink::<K>)
    }
}

/// Reactive-insertion `Component` for a signal handle whose value `O` is itself a
/// bundle: dropped onto an entity, it keeps the entity's `O` in sync with the
/// signal.
macro_rules! impl_bundle_component {
    ($handle:ident) => {
        impl<O: Bundle + Clone + Send + Sync + 'static> Component for $handle<O> {
            const STORAGE_TYPE: StorageType = StorageType::Table;
            type Mutability = Mutable;

            fn on_insert() -> Option<ComponentHook> {
                Some(handle_on_insert::<$handle<O>, O>)
            }

            fn on_replace() -> Option<ComponentHook> {
                Some(unbind_sink::<O>)
            }
        }
    };
}

impl_bundle_component!(DerivedSignal);
impl_bundle_component!(ObserverSignal);

/// `on_insert` hook shared by the signal-handle `Component` impls: spawn an
/// identity sink (insert the value's clone) and record the binding for teardown.
fn handle_on_insert<S, O>(mut world: DeferredWorld, ctx: HookContext)
where
    S: SignalRead<Value = O> + Component,
    O: Bundle + Clone + Send + Sync + 'static,
{
    let source = world.get::<S>(ctx.entity).unwrap().clone();
    let host = ctx.entity;
    let mut commands = world.commands();
    let sink = spawn_map_sink(
        &mut commands,
        host,
        source,
        Arc::new(|value: &O| value.clone()),
    );
    bind_sink::<O>(&mut commands, host, sink);
}

/// Records the sink entity created for a reactive insertion of bundle `K` on a
/// host, so it can be torn down when the insertion is replaced or the host is
/// despawned.
struct ReactiveBinding<K>(Entity, PhantomData<fn() -> K>);

impl<K: Send + Sync + 'static> Component for ReactiveBinding<K> {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;

    fn on_replace() -> Option<ComponentHook> {
        fn hook<K: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
            if let Some(sink) = world.get::<ReactiveBinding<K>>(ctx.entity).map(|b| b.0) {
                world
                    .commands()
                    .queue(move |world: &mut World| despawn_node(world, sink));
            }
        }
        Some(hook::<K>)
    }
}

fn bind_sink<K: Send + Sync + 'static>(commands: &mut Commands, host: Entity, sink: Entity) {
    commands
        .entity(host)
        .insert(ReactiveBinding::<K>(sink, PhantomData));
}

/// `on_replace` hook: drop the host's [`ReactiveBinding`] (which despawns the
/// sink). Guarded so it's a no-op if the host is already gone.
fn unbind_sink<K: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
    let host = ctx.entity;
    world.commands().queue(move |world: &mut World| {
        if let Ok(mut entity) = world.get_entity_mut(host) {
            entity.remove::<ReactiveBinding<K>>();
        }
    });
}

/// Reactive insertion for `Option`-valued signals: inserts the inner bundle when
/// `Some`, removes it when `None`, and leaves it untouched while `NotReady`.
pub trait SignalOption {
    type Item;

    fn option(&self) -> OptionSignal<Self::Item>;
}

impl<S, T> SignalOption for S
where
    S: SignalRead<Value = Option<T>>,
    T: Bundle + Clone + Send + Sync + 'static,
{
    type Item = T;

    fn option(&self) -> OptionSignal<T> {
        let source = self.clone();
        ReactiveInsert {
            spawn: Arc::new(move |commands: &mut Commands, host: Entity| {
                let source = source.clone();
                spawn_effect(commands, move |mut commands: Commands| {
                    match source.read() {
                        Ok(guard) => match &*guard {
                            Some(value) => {
                                let value = value.clone();
                                commands
                                    .entity(host)
                                    .reactive_cleanup::<T>()
                                    .try_insert(value);
                            }
                            None => {
                                commands.entity(host).remove::<T>();
                            }
                        },
                        Err(SignalError::NotReady) => {}
                    }
                })
            }),
            _marker: PhantomData,
        }
    }
}

// fn example(mut commands: Commands) {
//     #[derive(Component, Clone)]
//     #[component(immutable)]
//     struct Vec2 {
//         x: f32,
//         y: f32,
//     }

//     let xy = commands.signal(|data: Start<&Vec2>| data.clone());

//     let x = commands.derive({
//         let xy = xy.clone();
//         move || xy.get().x
//     });
//     let y = commands.derive({
//         let xy = xy.clone();
//         move || xy.get().y
//     });

//     let _area = commands.derive(move || x.get() * y.get());
// }

// fn counter(theme_entity: Entity, mut commands: Commands) -> impl Bundle {
//     #[derive(Component)]
//     #[component(immutable)]
//     struct ButtonCount(i32);

//     // Let's imagine there's some theme component.
//     #[derive(Component, Clone)]
//     #[component(immutable)]
//     struct Theme {
//         pub primary: String,
//         pub secondary: String,
//     }

//     // triggers every time the count changes via "query observers"
//     let count = commands.signal(|count: Start<&ButtonCount>| count.0);
//     let theme = commands
//         .signal(|theme: Start<&Theme>| theme.clone())
//         .watch_entity(theme_entity);

//     // Here, we have a special reactive system that tracks signals,
//     // re-running any time they change. This is very feasible -- I've
//     // already implemented it in my current crate.
//     let background_color = commands.derive({
//         let count = count.clone();
//         move || match count.get() {
//             0 => theme.get().primary,
//             _ => theme.get().secondary,
//         }
//     });

//     Ok((
//         Div,
//         ButtonCount(0),
//         // This would have the `count` observer watch this specific entity,
//         // since in a bundle context we don't have one yet.
//         count.watch_bundle(),
//         background_color.map(|c| Style::new(format!("background-color: {c}")),
//         children![
//             // An FRP-style insertion, re-allocating a string every time
//             count.map(|c| Text::new(format!("count: {c}"))),
//             // An imperative-style API, which we could presumably support
//             count.react(|count: i32, data: Data<&mut Text>| {
//                 data.0.clear();
//                 write!(&mut data.0, "count: {count}");
//             }),
//             (
//                 Div,
//                 text!("+"),
//                 // `GetUp` would just be a wrapper around
//                 // two-query up traversal, nothing special
//                 ev::click(|count: GetUp<&ButtonCount>| {
//                     let new_count = count.0 + 1;
//                     count.set(ButtonCount(new_count));
//                 })
//             ),
//             (
//                 Div,
//                 text!("-"),
//                 ev::click(|count: GetUp<&ButtonCount>| {
//                     let new_count = count.0 - 1;
//                     count.set(ButtonCount(new_count));
//                 })
//             )
//         ],
//     ))
// }

#[cfg(test)]
mod test {
    use super::*;
    use crate::cleanup::CleanupPlugin;
    use bevy_app::prelude::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    /// Simulates an input firing: overwrite its cached value, then enqueue it as
    /// a changed source so the next flush marks its subscribers.
    fn drive_input<O: Send + Sync + 'static>(app: &mut App, input: &DerivedSignal<O>, value: O) {
        *input.value.write().unwrap() = Some(value);
        app.world_mut()
            .resource_mut::<PendingDirty>()
            .0
            .push(input.inner.entity);
    }

    fn signal_node_count(world: &mut World) -> usize {
        let mut q = world.query::<&SignalSystem>();
        q.iter(world).count()
    }

    /// `xy → {x, y} → area`. When `xy` changes, `area` must recompute exactly
    /// once (not once per changed arm) and see a consistent view of both.
    #[test]
    fn diamond_settles_shared_node_once() {
        let mut app = App::new();
        app.add_plugins(Signal2Plugin);

        let x_runs = Arc::new(AtomicUsize::new(0));
        let area_runs = Arc::new(AtomicUsize::new(0));

        let world = app.world_mut();
        let mut commands = world.commands();

        // Apex driven manually via `drive_input` (stands in for an observer input).
        let xy = commands.derive(|| Ok(2.0_f32));
        let x = {
            let xy = xy.clone();
            let runs = x_runs.clone();
            commands.derive(move || {
                runs.fetch_add(1, Ordering::Relaxed);
                Ok(xy.get()?)
            })
        };
        let y = {
            let xy = xy.clone();
            commands.derive(move || Ok(xy.get()?))
        };
        let area = {
            let (x, y, runs) = (x.clone(), y.clone(), area_runs.clone());
            commands.derive(move || {
                runs.fetch_add(1, Ordering::Relaxed);
                Ok(x.get()? * y.get()?)
            })
        };

        app.update();

        // Initial evaluation: everything ran once, values flowed through.
        assert_eq!(x_runs.load(Ordering::Relaxed), 1);
        assert_eq!(area_runs.load(Ordering::Relaxed), 1);
        assert_eq!(area.get(), Ok(4.0));

        // Change the apex and flush.
        drive_input(&mut app, &xy, 3.0);
        app.update();

        // Both arms re-ran, but the shared node recomputed only once more.
        assert_eq!(x_runs.load(Ordering::Relaxed), 2);
        assert_eq!(area_runs.load(Ordering::Relaxed), 2);
        assert_eq!(area.get(), Ok(9.0));

        // A flush with nothing pending is a no-op — no idle recomputation.
        app.update();
        assert_eq!(area_runs.load(Ordering::Relaxed), 2);
    }

    /// A node outside the changed subgraph is never recomputed.
    #[test]
    fn unrelated_node_is_not_recomputed() {
        let mut app = App::new();
        app.add_plugins(Signal2Plugin);

        let dependent_runs = Arc::new(AtomicUsize::new(0));
        let unrelated_runs = Arc::new(AtomicUsize::new(0));

        let world = app.world_mut();
        let mut commands = world.commands();

        let source = commands.derive(|| Ok(1.0_f32));
        let _dependent = {
            let source = source.clone();
            let runs = dependent_runs.clone();
            commands.derive(move || {
                runs.fetch_add(1, Ordering::Relaxed);
                Ok(source.get()? + 1.0)
            })
        };
        let _unrelated = {
            let runs = unrelated_runs.clone();
            commands.derive(move || {
                runs.fetch_add(1, Ordering::Relaxed);
                Ok(42.0_f32)
            })
        };

        app.update();
        assert_eq!(dependent_runs.load(Ordering::Relaxed), 1);
        assert_eq!(unrelated_runs.load(Ordering::Relaxed), 1);

        drive_input(&mut app, &source, 5.0);
        app.update();

        // Only the dependent chain re-ran.
        assert_eq!(dependent_runs.load(Ordering::Relaxed), 2);
        assert_eq!(unrelated_runs.load(Ordering::Relaxed), 1);
    }

    /// `NotReady` from a source propagates through `?`, and clears once the
    /// source becomes ready.
    #[test]
    fn not_ready_propagates() {
        let mut app = App::new();
        app.add_plugins(Signal2Plugin);

        let world = app.world_mut();
        let mut commands = world.commands();

        let input = commands.derive(|| Ok(0.0_f32));
        let gated = {
            let input = input.clone();
            commands.derive(move || {
                let v = input.get()?;
                if v > 0.0 {
                    Ok(v)
                } else {
                    Err(SignalError::NotReady)
                }
            })
        };
        let dependent = {
            let gated = gated.clone();
            commands.derive(move || Ok(gated.get()? + 1.0))
        };

        app.update();
        assert_eq!(gated.get(), Err(SignalError::NotReady));
        assert_eq!(dependent.get(), Err(SignalError::NotReady));

        drive_input(&mut app, &input, 7.0);
        app.update();
        assert_eq!(gated.get(), Ok(7.0));
        assert_eq!(dependent.get(), Ok(8.0));
    }

    #[derive(Component, Clone, PartialEq, Debug)]
    struct Tag(u32);

    #[derive(Component, Clone, PartialEq, Debug)]
    struct Doubled(u32);

    /// A `DerivedSignal<Bundle>` dropped on an entity reactively (re)inserts its
    /// value, and the sink is torn down when the host is despawned.
    #[test]
    fn reactive_bundle_insertion_and_teardown() {
        let mut app = App::new();
        app.add_plugins((Signal2Plugin, CleanupPlugin));

        let world = app.world_mut();
        let mut commands = world.commands();

        let input = commands.derive(|| Ok(1u32));
        let tag = {
            let input = input.clone();
            commands.derive(move || Ok(Tag(input.get()?)))
        };
        let host = commands.spawn(tag.clone()).id();

        app.update();
        assert_eq!(app.world().get::<Tag>(host), Some(&Tag(1)));

        drive_input(&mut app, &input, 5);
        app.update();
        assert_eq!(app.world().get::<Tag>(host), Some(&Tag(5)));

        // Teardown: despawning the host removes its sink node.
        let before = signal_node_count(app.world_mut());
        app.world_mut().despawn(host);
        app.update();
        assert_eq!(signal_node_count(app.world_mut()), before - 1);
    }

    /// `.map(|&v| ...)` inserts (and keeps current) a bundle mapped by reference.
    #[test]
    fn map_by_reference_inserts() {
        let mut app = App::new();
        app.add_plugins((Signal2Plugin, CleanupPlugin));

        let world = app.world_mut();
        let mut commands = world.commands();

        let input = commands.derive(|| Ok(2u32));
        let host = commands.spawn(input.map(|v: &u32| Doubled(v * 2))).id();

        app.update();
        assert_eq!(app.world().get::<Doubled>(host), Some(&Doubled(4)));

        drive_input(&mut app, &input, 10);
        app.update();
        assert_eq!(app.world().get::<Doubled>(host), Some(&Doubled(20)));
    }

    /// `.option()` inserts the inner bundle on `Some` and removes it on `None`.
    #[test]
    fn option_removes_on_none() {
        let mut app = App::new();
        app.add_plugins((Signal2Plugin, CleanupPlugin));

        let world = app.world_mut();
        let mut commands = world.commands();

        let input = commands.derive(|| Ok(1u32));
        let maybe = {
            let input = input.clone();
            commands.derive(move || {
                let v = input.get()?;
                Ok((v > 0).then_some(Tag(v)))
            })
        };
        let host = commands.spawn(maybe.option()).id();

        app.update();
        assert_eq!(app.world().get::<Tag>(host), Some(&Tag(1)));

        drive_input(&mut app, &input, 0);
        app.update();
        assert_eq!(app.world().get::<Tag>(host), None);
    }
}
