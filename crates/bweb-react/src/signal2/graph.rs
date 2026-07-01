//! The reactive graph engine: world-resident node state (edges, status,
//! systems) and the push-based flush — mark → topological settle → bounded
//! fixpoint.

use bevy_ecs::{entity::EntityIndexSet, prelude::*, system::SystemId};
use bevy_platform::collections::{HashMap, HashSet};

use super::reactive_context::ReactiveContext;

/// Input nodes whose value changed since the last flush.
///
/// Fed by the query observers backing `ObserverSignal`s (and by `derive`/`memo`
/// sinks that trip observers); drained by the flush, which marks each source's
/// subscriber subgraph (see [`mark_from_source`]).
#[derive(Resource, Default)]
pub(crate) struct PendingDirty(pub(crate) Vec<Entity>);

/// Nodes whose value changed during the current flush pass.
///
/// Seeded with the inputs that fired, then extended as nodes recompute (each
/// node's sink inserts itself when it should propagate). A `Check` node
/// recomputes only if one of its sources is in this set. Cleared at the start of
/// each fixpoint pass.
///
/// Named `Changed*` to avoid colliding with [`bevy_ecs::prelude::Changed`].
#[derive(Resource, Default)]
pub(crate) struct ChangedNodes(pub(crate) HashSet<Entity>);

/// Diagnostics for the last [`flush`], published under the `dev` feature so
/// [`run_react_schedule`](super::run_react_schedule) can report how much work a
/// frame's propagation took.
#[cfg(feature = "dev")]
#[derive(Resource, Default)]
pub(crate) struct FlushMetrics {
    /// Number of fixpoint passes that actually settled the graph last flush. A
    /// well-formed graph settles in one; more means a sink tripped an input
    /// (or a mid-flush rewire) and forced another round.
    pub(crate) passes: usize,
}

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
pub struct Subscribers(EntityIndexSet);

/// Backward edges: the source entities this node reads.
///
/// Rewritten by [`rewire_edges`] after each evaluation from the set collected by
/// [`ReactiveContext`]; the reverse of [`Subscribers`].
#[derive(Component, Default)]
pub struct Sources(EntityIndexSet);

/// How a derived node recomputes. Input/source nodes (driven by a query
/// observer) have no `SignalSystem`.
#[derive(Component, Clone, Copy)]
pub struct SignalSystem(pub(crate) SystemId);

/// Marks a node as *polled*: re-evaluated on every flush pass regardless of
/// whether any source changed. Its sink is still a memo sink, so propagation
/// stays value-gated — a poll whose value is stable prunes its subgraph. Lets
/// signals track world state that emits no lifecycle event for observers.
#[derive(Component)]
pub(crate) struct Polled;

/// A type-erased evaluator for a closure-based node: runs the user closure inside
/// a reactive context, writes the value cell, and reports `(changed, sources)`.
pub(crate) type ClosureEval = Box<dyn FnMut() -> (bool, EntityIndexSet) + Send + Sync>;

/// A `derive`/`memo` node's evaluator. Closure nodes skip Bevy's system
/// machinery entirely — most derived signals only read other signals, so a plain
/// call is far cheaper than registering and running a one-shot system.
#[derive(Component)]
pub(crate) struct SignalClosure(pub(crate) ClosureEval);

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
pub(crate) fn flush(world: &mut World) {
    #[cfg(feature = "dev")]
    let mut passes = 0usize;

    for _ in 0..REACTION_LIMIT {
        // Start each pass from a clean change set: this discards marks left by
        // initial (non-flush) evaluations and by the previous pass, so every pass
        // is a fresh propagation from its own inputs.
        world.resource_mut::<ChangedNodes>().0.clear();

        let (inputs, mut active) = drain_and_mark(world);
        // Poll nodes re-run every pass, regardless of whether a source changed.
        mark_polled(world, &mut active);
        if active.is_empty() {
            break;
        }

        #[cfg(feature = "dev")]
        {
            passes += 1;
        }

        // The inputs that fired have, by definition, changed.
        world.resource_mut::<ChangedNodes>().0.extend(inputs);

        settle_active(world, active);

        if world.resource::<PendingDirty>().0.is_empty() {
            break;
        }
    }

    #[cfg(feature = "dev")]
    if let Some(mut metrics) = world.get_resource_mut::<FlushMetrics>() {
        metrics.passes = passes;
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
    let stack: Vec<(Entity, NodeStatus)> = world
        .get::<Subscribers>(source)
        .map(|subs| subs.0.iter().map(|&e| (e, NodeStatus::Dirty)).collect())
        .unwrap_or_default();
    mark_stack(world, stack, active);
}

/// Force-marks every [`Polled`] node `Dirty` (and propagates `Check` to its
/// subscribers) so it re-evaluates this pass even without a source change.
fn mark_polled(world: &mut World, active: &mut Vec<Entity>) {
    let polls: Vec<Entity> = {
        let mut q = world.query_filtered::<Entity, With<Polled>>();
        q.iter(world).collect()
    };
    for node in polls {
        mark_stack(world, vec![(node, NodeStatus::Dirty)], active);
    }
}

/// The shared marking loop (see [`mark_from_source`]): raises each node's status,
/// pushing `Check` to its subscribers only when it first leaves `Clean`, and
/// records each newly-active node in `active` exactly once.
fn mark_stack(world: &mut World, mut stack: Vec<(Entity, NodeStatus)>, active: &mut Vec<Entity>) {
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

        let subscribers = world.get::<Subscribers>(node);

        for sub in subscribers.iter().flat_map(|s| s.0.iter()) {
            if let Some(degree) = in_degree.get_mut(sub) {
                *degree = degree.saturating_sub(1);
                if *degree == 0 {
                    ready.push(*sub);
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

/// Settles a single node: recomputes it if it must, then resets it to `Clean`.
///
/// A `Dirty` node always recomputes. A `Check` node recomputes only if one of
/// its sources landed in [`ChangedNodes`] this pass. Whether a recompute
/// *propagates* (lands the node in [`ChangedNodes`]) is decided by the node's own
/// sink — unconditional for `derive`, value-gated for `memo`.
fn settle_node(world: &mut World, node: Entity) {
    let status = world
        .get::<NodeStatus>(node)
        .copied()
        .unwrap_or(NodeStatus::Clean);

    let should_run = match status {
        NodeStatus::Dirty => true,
        NodeStatus::Check => {
            let sources = world.get::<Sources>(node);
            let changed = &world.resource::<ChangedNodes>().0;
            sources
                .iter()
                .flat_map(|s| s.0.iter())
                .any(|src| changed.contains(src))
        }
        NodeStatus::Clean => false,
    };

    if should_run {
        evaluate_node(world, node);
    }

    if let Some(mut status) = world.get_mut::<NodeStatus>(node) {
        *status = NodeStatus::Clean;
    }
}

/// Evaluates a node while collecting its source reads, then reconciles its edges.
///
/// - Closure nodes ([`SignalClosure`], i.e. `derive`/`memo`) are called directly
///   — no Bevy system — and report whether their value moved; [`evaluate_node`]
///   marks them in [`ChangedNodes`] accordingly.
/// - System nodes ([`SignalSystem`], i.e. reactive sinks and `poll`) are run via
///   `run_system`; their own sink writes the value and marks change propagation.
/// - Input nodes (neither component) are a no-op — driven by their query observer.
pub(crate) fn evaluate_node(world: &mut World, node: Entity) {
    if let Some((changed, sources)) = world
        .get_mut::<SignalClosure>(node)
        .map(|mut closure| (closure.bypass_change_detection().0)())
    {
        if changed {
            world.resource_mut::<ChangedNodes>().0.insert(node);
        }
        rewire_edges(world, node, sources);
        return;
    }

    if let Some(SignalSystem(system)) = world.get::<SignalSystem>(node).copied() {
        let (result, sources) = ReactiveContext::collect(|| world.run_system(system));
        if let Err(e) = result {
            log::error!("Failed to run signal system: {e}");
        }
        rewire_edges(world, node, sources);
    }
}

/// Reconciles `node`'s dependency edges after an evaluation.
///
/// `new_sources` is the (possibly duplicated) set of source entities read during
/// the run. This deduplicates them, then updates both directions in place:
/// removes `node` from the [`Subscribers`] of sources it no longer reads, adds it
/// to those it newly reads, and stores the deduped set as `node`'s [`Sources`].
fn rewire_edges(world: &mut World, node: Entity, new_set: EntityIndexSet) {
    let old_set = world
        .get::<Sources>(node)
        .map(|s| s.0.clone())
        .unwrap_or_default();

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
                subs.0.insert(node);
            }
        }
    }

    if let Some(mut sources) = world.get_mut::<Sources>(node) {
        sources.0 = new_set;
    }
}

/// Spawns a graph node whose system runs for side effects (not a readable
/// value). Unlike `derive` the system may use `Commands` (it is not a
/// `ReadOnlySystem`) and there is no value cell. Used for reactive sinks —
/// component insertion, mapping, `option`.
pub(crate) fn spawn_effect<Sys, M>(commands: &mut Commands, system: Sys) -> Entity
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
pub(crate) fn despawn_node(world: &mut World, node: Entity) {
    rewire_edges(world, node, EntityIndexSet::new());
    if let Some(SignalSystem(system)) = world.get::<SignalSystem>(node).copied() {
        let _ = world.unregister_system(system);
    }
    world.despawn(node);
}
