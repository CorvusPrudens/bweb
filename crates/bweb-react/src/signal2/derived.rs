//! Derived signals: closures over any number of other signals.
//!
//! A source signal's subscribers are dispatched straight off the scan, which is
//! right for a one-hop effect but wrong for a node with several inputs — it
//! would run once per changed input, and a diamond would run it once on stale
//! data. So the scan doesn't evaluate a derived node at all; it only marks it,
//! and this module's single pass evaluates each marked node once, cheapest
//! level first.

use bevy_ecs::{lifecycle::HookContext, prelude::*, system::InMut, world::DeferredWorld};
use smallvec::SmallVec;

use crate::signal2::{
    ReactError, ReactiveWork,
    source::{SignalStore, SourceSignal, UnsubscribeFn, Watching},
};

/// Derived nodes created since the last pass.
///
/// A node is spawned outside any pass (or by a subscriber mid-pass), so it has
/// to be seeded into the next pass's dirty list to get its first evaluation.
/// The schedule runner drains this — the reactive systems themselves stay
/// read-only.
#[derive(Resource, Default)]
pub(crate) struct PendingNodes(pub(crate) Vec<Entity>);

/// What a derived node's evaluator is handed.
pub struct DerivedContext<'a, 'w, 's, 'cw, 'cs> {
    /// The node's own entity — also where its value lives.
    pub node: Entity,
    pub store: &'a SignalStore<'w, 's>,
    /// Edges this node already has, so a re-evaluation can tell a new read from
    /// one it reported last time.
    pub(crate) sources: Option<&'a NodeSources>,
    pub work: &'a mut ReactiveWork,
    pub commands: Commands<'cw, 'cs>,
}

type DerivedEval =
    Box<dyn for<'a, 'w, 's, 'cw, 'cs> Fn(DerivedContext<'a, 'w, 's, 'cw, 'cs>) + Send + Sync>;

#[derive(Component)]
#[component(on_add = DerivedNode::on_add, on_replace = DerivedNode::on_replace)]
pub(crate) struct DerivedNode {
    eval: DerivedEval,
}

impl DerivedNode {
    fn on_add(mut world: DeferredWorld, ctx: HookContext) {
        world.resource_mut::<PendingNodes>().0.push(ctx.entity);
    }

    /// Tear the node's edges down when it goes away.
    ///
    /// `on_replace` rather than `on_remove` because it is the one hook that fires
    /// for all three ways a node can end — overwritten, removed, despawned — and
    /// on a despawn it still runs while every component is present, so the
    /// node's `NodeSources` can be read here.
    fn on_replace(mut world: DeferredWorld, ctx: HookContext) {
        let node = ctx.entity;
        let Some(sources) = world
            .get::<NodeSources>(node)
            .map(|sources| sources.0.clone())
        else {
            return;
        };
        if sources.is_empty() {
            return;
        }

        world.commands().queue(move |world: &mut World| {
            unsubscribe_all(world, node, &sources);
        });
    }
}

/// Distance from the nearest plain component, computed as edges are wired.
///
/// Sorting a pass's dirty list by level is the cheap stand-in for a topological
/// sort: it costs one `sort_unstable` over the dirty nodes instead of a
/// per-flush in-degree table, and it is enough to keep a node from evaluating
/// before an input that is dirty in the same pass.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct NodeLevel(pub u16);

/// How far a level raise will walk downstream before giving up.
///
/// A level can only ever rise, so an acyclic graph terminates after one visit
/// per reachable node. Hitting the cap means the graph has a cycle, in which
/// case the walk would otherwise climb one level at a time to `u16::MAX`.
const LEVEL_PROPAGATION_LIMIT: usize = 4096;

/// One edge from a derived node to something it reads.
#[derive(Clone, Copy)]
pub(crate) struct NodeSource {
    /// The signal entity the node read through.
    pub(crate) signal: Entity,
    /// What that signal was pointing at when the edge was wired — the entity
    /// actually carrying the `SubscriberSet` this node's mark lives in.
    pub(crate) target: Entity,
    /// Drops that mark. A plain fn pointer because the derived pass is generic
    /// over nothing: it has to undo a subscription without knowing `D`.
    pub(crate) unsubscribe: UnsubscribeFn,
}

/// What this node has already subscribed to.
///
/// Subscription is lazy: a node's evaluation reports what it read, and the diff
/// against this list is what gets wired and unwired. A branch that starts
/// reading a new signal picks up its edge on that run; one that stops reading a
/// signal drops the edge on the same run, so a node is not woken by an input it
/// no longer looks at.
#[derive(Component, Default)]
pub(crate) struct NodeSources(pub(crate) SmallVec<[NodeSource; 4]>);

impl NodeSources {
    pub(crate) fn contains(&self, signal: Entity) -> bool {
        self.0.iter().any(|source| source.signal == signal)
    }

    /// Whether any remaining edge still resolves to `target` — i.e. whether the
    /// mark closure on `target` is still needed.
    pub(crate) fn watches(&self, target: Entity) -> bool {
        self.0.iter().any(|source| source.target == target)
    }
}

/// Derived nodes subscribed to this entity: the reverse of [`NodeSources`].
///
/// Only levels need this. A subscription is otherwise an opaque closure, and a
/// node whose input deepens after the fact has no other way to find the nodes
/// downstream of it that must deepen too.
#[derive(Component, Default)]
pub(crate) struct NodeSubscribers(pub(crate) SmallVec<[Entity; 4]>);

/// Raise `node` to at least `to`, and everything downstream of it to match.
///
/// Levels are discovered, not declared — a node can evaluate before the input it
/// reads has a level of its own, and a node deep in a chain can gain a new,
/// deeper input long after its own subscribers were wired. Without this walk
/// that raise would stop at the node itself and leave its subscribers claiming
/// to be shallower than their input, which costs a settle pass every time they
/// both go dirty.
pub(crate) fn raise_level(world: &mut World, node: Entity, to: u16) {
    let mut stack: SmallVec<[(Entity, u16); 8]> = SmallVec::new();
    stack.push((node, to));

    let mut budget = LEVEL_PROPAGATION_LIMIT;
    while let Some((current, want)) = stack.pop() {
        if budget == 0 {
            log::warn!(
                "signal2: raising the level of {node} walked more than \
                 {LEVEL_PROPAGATION_LIMIT} nodes without settling; the derived graph \
                 most likely contains a cycle"
            );
            return;
        }
        budget -= 1;

        {
            // Not a derived node, or already at least this deep — either way the
            // walk stops, which is what makes the whole thing terminate.
            let Some(mut level) = world.get_mut::<NodeLevel>(current) else {
                continue;
            };
            if level.0 >= want {
                continue;
            }
            level.0 = want;
        }

        if let Some(subscribers) = world.get::<NodeSubscribers>(current) {
            let next = want.saturating_add(1);
            stack.extend(subscribers.0.iter().map(|&node| (node, next)));
        }
    }
}

/// Drop the edge from `node` to `signal`, and the mark behind it if that was the
/// last edge pointing at the same target.
pub(crate) fn unsubscribe_source(world: &mut World, node: Entity, signal: Entity) {
    let dropped = {
        let Some(mut sources) = world.get_mut::<NodeSources>(node) else {
            return;
        };
        let Some(index) = sources.0.iter().position(|source| source.signal == signal) else {
            return;
        };

        let dropped = sources.0.remove(index);
        // Two signals can point at the same entity, in which case they share one
        // mark closure and the other edge is still using it.
        if sources.watches(dropped.target) {
            return;
        }
        dropped
    };

    (dropped.unsubscribe)(world, dropped.target, node);
}

/// Drop every edge `node` holds, for the case where the node itself is going
/// away and there is no `NodeSources` left to diff against.
fn unsubscribe_all(world: &mut World, node: Entity, sources: &[NodeSource]) {
    let mut done: SmallVec<[Entity; 4]> = SmallVec::new();
    for source in sources {
        if done.contains(&source.target) {
            continue;
        }
        done.push(source.target);
        (source.unsubscribe)(world, source.target, node);
    }

    // The node may have only lost the `DerivedNode` component rather than been
    // despawned, and a re-insert has to start from a clean edge list.
    if let Ok(mut node) = world.get_entity_mut(node)
        && let Some(mut sources) = node.get_mut::<NodeSources>()
    {
        sources.0.clear();
    }
}

pub(crate) fn spawn_derive<F, O>(commands: &mut Commands, eval: F) -> SourceSignal<&'static O>
where
    F: Fn(&SignalStore) -> Result<O, ReactError> + Send + Sync + 'static,
    O: Component,
{
    let node = commands.spawn_empty().id();
    commands.entity(node).insert((
        NodeLevel(0),
        NodeSources::default(),
        // A derived node is its own source, so `SourceSignal::get` and
        // `subscribe_derived` treat it exactly like a watched entity.
        Watching(node),
        DerivedNode {
            eval: make_eval(eval),
        },
    ));

    SourceSignal::from_entity(node)
}

fn make_eval<F, O>(eval: F) -> DerivedEval
where
    F: Fn(&SignalStore) -> Result<O, ReactError> + Send + Sync + 'static,
    O: Component,
{
    Box::new(move |mut ctx: DerivedContext| {
        let node = ctx.node;

        ctx.store.clear_reads();
        let value = eval(ctx.store);
        let reads = ctx.store.take_reads();

        // The node's dependencies are whatever it just read, so the edge set is
        // the diff between that and what it already has.
        //
        // A node that took the same path through its closure reports the same
        // reads in the same order, which is the overwhelmingly common case and
        // the one worth spending a branch on: comparing the two lists elementwise
        // settles it in one pass, where the diff below is quadratic in the number
        // of inputs. Wiring an edge costs a handful of random world lookups, so
        // this path has to stay off the steady state entirely.
        let known = ctx.sources;
        let unchanged = known.is_some_and(|known| {
            known.0.len() == reads.len()
                && known
                    .0
                    .iter()
                    .zip(reads.iter())
                    .all(|(source, (signal, _))| source.signal == *signal)
        });

        if !unchanged {
            let fresh = reads
                .iter()
                .copied()
                .filter(|(signal, _)| !known.is_some_and(|known| known.contains(*signal)))
                .collect::<Vec<_>>();

            // A read this node used to do and didn't this time. Dropping the edge
            // is what keeps a branch that took the other arm from being woken by
            // an input it no longer looks at.
            let stale = known.map_or_else(Vec::new, |known| {
                known
                    .0
                    .iter()
                    .filter(|source| !reads.iter().any(|(signal, _)| *signal == source.signal))
                    .map(|source| source.signal)
                    .collect::<Vec<_>>()
            });

            // Reads can be reordered without the set changing, in which case
            // there is nothing to wire either way.
            if !fresh.is_empty() || !stale.is_empty() {
                ctx.commands.queue(move |world: &mut World| {
                    // Stale first: an edge being replaced by one pointing at the
                    // same target must release the shared mark before the new one
                    // claims it, or the release would take the new mark with it.
                    for signal in stale {
                        unsubscribe_source(world, node, signal);
                    }
                    for (signal, subscribe) in fresh {
                        subscribe(world, signal, node);
                    }
                });
            }
        }

        match value {
            Ok(value) => {
                ctx.commands.entity(node).insert(value);
            }
            Err(e) => {
                // TODO: handle error cases properly
                log::error!("Failed to evaluate node: {e}");
            }
        }
    })
}

/// Evaluate every node marked dirty this pass, once each, lowest level first.
pub(crate) fn run_derived_nodes(
    InMut(work): InMut<ReactiveWork>,
    nodes: Query<(&DerivedNode, Option<&NodeSources>)>,
    levels: Query<&NodeLevel>,
    store: SignalStore,
    mut commands: Commands,
) {
    if work.dirty.is_empty() {
        return;
    }

    let mut dirty = core::mem::take(&mut work.dirty);

    // Dedup first, keeping first-marked order: this is what collapses a
    // diamond's two marks into one evaluation.
    work.seen.clear();
    dirty.retain(|node| work.seen.insert(*node));

    // Then a *stable* sort on level alone. Ties keep insertion order, which for
    // a node's first pass is spawn order — and since signals are built bottom
    // up, that is already dependency order, so levels come out right the first
    // time rather than converging over passes.
    dirty.sort_by_key(|node| levels.get(*node).copied().unwrap_or_default());

    for node in dirty {
        let Ok((derived, sources)) = nodes.get(node) else {
            // Despawned between being marked and being settled.
            continue;
        };

        work.dispatched += 1;
        (derived.eval)(DerivedContext {
            node,
            store: &store,
            sources,
            work: &mut *work,
            commands: commands.reborrow(),
        });
    }
}
