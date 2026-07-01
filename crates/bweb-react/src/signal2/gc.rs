//! Arc-strong-count garbage collection for readable signal nodes.
//!
//! Every readable signal (`signal` / `derive` / `memo` / `poll` / `track`) spawns a
//! graph node whose value/shared `Arc` lives in the handle. That `Arc` is also cloned
//! into exactly one intrinsic, handle-independent holder (the node's evaluator closure,
//! observer sink, registered system, or the `track` registry writer) plus the
//! [`SignalGc`] probe below. So while any external handle (or a downstream closure that
//! captured it) is alive the strong count stays above the intrinsic baseline of `2`;
//! once the last one drops, [`gc_pass`] collects the node via
//! [`despawn_node`](super::graph::despawn_node).
//!
//! This mirrors the v1 framework's `SignalGc` / `gc_pass` (`src/signal/mod.rs`): a
//! strong-holding probe, a per-constructor rest count, and a [`SweepFrequency`]-gated
//! sweep in `Last`. Cross-signal edges never pin their source — liveness flows purely
//! through handle captures — so an abandoned subgraph collects leaf-first over
//! successive sweeps.

use bevy_ecs::prelude::*;
use bevy_platform::time::Instant;
use std::any::Any;
use std::sync::Arc;

use super::graph::despawn_node;

/// Strong-count probe for a readable signal node. Holds a type-erased strong clone of
/// the node's value/shared `Arc`; the node is collected once the strong count falls to
/// `rest` (no external handle or downstream capture remains).
#[derive(Component)]
pub(crate) struct SignalGc {
    probe: Arc<dyn Any + Send + Sync>,
    rest: usize,
}

impl SignalGc {
    /// A probe over `arc` with the given rest baseline. Every signal2 node uses
    /// `rest = 2` (its one intrinsic clone + this probe).
    pub(crate) fn new<T: Send + Sync + 'static>(arc: &Arc<T>, rest: usize) -> Self {
        Self {
            probe: arc.clone(),
            rest,
        }
    }
}

/// Number of live signal2 reactive nodes (every readable node carries a
/// [`SignalGc`]). Diagnostic; see [`crate::reactive_node_counts`].
pub(crate) fn live_node_count(world: &mut World) -> usize {
    world.query::<&SignalGc>().iter(world).count()
}

/// How long [`gc_pass`] waits between sweeps. Defaults to one second; tests set
/// `Duration::ZERO` to force a sweep every frame.
#[derive(Resource)]
pub struct SweepFrequency(pub core::time::Duration);

impl Default for SweepFrequency {
    fn default() -> Self {
        Self(core::time::Duration::from_secs(10))
    }
}

/// Collects readable nodes whose value `Arc` has dropped to its intrinsic baseline.
/// Runs in `Last`, after `ReactSchedule` has settled for the frame, and only once per
/// [`SweepFrequency`] interval. Collection is deferred through `Commands` so it never
/// races a mid-flush borrow.
pub(super) fn gc_pass(
    nodes: Query<(Entity, &SignalGc)>,
    frequency: Res<SweepFrequency>,
    mut last_sweep: Local<Option<Instant>>,
    mut commands: Commands,
) {
    let now = Instant::now();
    let last = *last_sweep.get_or_insert(now);
    if now.duration_since(last) < frequency.0 {
        return;
    }
    *last_sweep = Some(now);

    #[cfg(feature = "dev")]
    let mut collecting = 0usize;

    for (node, gc) in &nodes {
        if Arc::strong_count(&gc.probe) <= gc.rest {
            #[cfg(feature = "dev")]
            {
                collecting += 1;
            }
            commands.queue(move |world: &mut World| despawn_node(world, node));
        }
    }

    #[cfg(feature = "dev")]
    log::debug!(
        "signal2 census: {} live node(s), collecting {collecting}",
        nodes.iter().len(),
    );
}
