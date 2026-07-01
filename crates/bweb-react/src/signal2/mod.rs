//! A push-based reactive signal graph over `bevy_ecs`.
//!
//! Modules:
//! - [`graph`] — the engine: world-resident node state and the flush
//!   (mark → topological settle → bounded fixpoint).
//! - [`handle`] — signal handles ([`DerivedSignal`], [`ObserverSignal`]) and the
//!   fallible read API ([`SignalRead`]).
//! - [`signal`] — the [`SignalExt`] constructors (`signal` / `derive` / `memo`).
//! - [`track`] — change-scanning input sources ([`Track::track`]) for components
//!   that mutate in place (e.g. `Children`).
//! - [`insert`] — reactive Bundle insertion ([`SignalMap`], [`SignalOption`]).
//! - [`error`], [`reactive_context`] — the read error type and the
//!   thread-local source-collection used during evaluation.

use bevy_app::prelude::*;

mod error;
mod gc;
mod graph;
mod handle;
mod insert;
mod list;
mod reactive_context;
mod signal;
mod track;

#[cfg(test)]
mod tests;

use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{IntoScheduleConfigs, ScheduleLabel, SystemSet};
pub(crate) use gc::live_node_count;
use gc::gc_pass;
#[cfg(feature = "dev")]
use graph::FlushMetrics;
use graph::{ChangedNodes, PendingDirty, flush};
use track::TrackedTypes;

pub use error::{SignalError, SignalReadGuard, SignalResult};
pub use gc::SweepFrequency;
pub use graph::{NodeStatus, SignalSystem, Sources, Subscribers};
pub use handle::{DerivedSignal, ObserverSignal, Signal, SignalRead, WatchBundle};
pub use insert::{MappedSignal, OptionSignal, ReactiveInsert, SignalMap, SignalOption};
pub use list::{ReactiveList, ReactiveListExt};
pub use signal::SignalExt;
pub use track::{Track, TrackWatchBundle, TrackedSignal};

/// The schedule housing signal2's evaluation: change scanners (`Scan`) then the
/// graph settle (`Settle`). Run once per frame from `PostUpdate`. Scanners are
/// added to it on demand by [`Track::track`].
#[derive(ScheduleLabel, PartialEq, Eq, Clone, Debug, Hash)]
pub struct ReactSchedule;

#[derive(SystemSet, PartialEq, Eq, Clone, Debug, Hash)]
pub enum ReactiveSystems {
    /// Change scanners feed `PendingDirty` from `Changed<T>` (see [`track`]).
    Scan,
    /// The graph flush drains `PendingDirty` and settles the DAG.
    Settle,
}

/// Wires up the signal2 reactive runtime.
pub struct Signal2Plugin;

impl Plugin for Signal2Plugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "dev")]
        app.init_resource::<FlushMetrics>();

        app.init_resource::<PendingDirty>()
            .init_resource::<ChangedNodes>()
            .init_resource::<TrackedTypes>()
            .init_resource::<SweepFrequency>()
            .init_schedule(ReactSchedule)
            .configure_sets(
                ReactSchedule,
                ReactiveSystems::Scan.before(ReactiveSystems::Settle),
            )
            .add_systems(ReactSchedule, flush.in_set(ReactiveSystems::Settle))
            .add_systems(PostUpdate, run_react_schedule)
            .add_systems(Last, gc_pass);
    }
}

/// Runs [`ReactSchedule`] once per frame. `flush` already iterates to quiescence
/// internally, so a single run per frame suffices.
#[cfg(not(feature = "dev"))]
fn run_react_schedule(world: &mut World) {
    world.run_schedule(ReactSchedule);
}

/// `dev` build: times the whole [`ReactSchedule`] run (change scanners + graph
/// settle) and reports the elapsed wall time alongside the fixpoint pass count
/// published by [`flush`]. Only logs on frames that did work, so idle frames stay
/// quiet.
#[cfg(feature = "dev")]
fn run_react_schedule(world: &mut World) {
    let start = bevy_platform::time::Instant::now();
    world.run_schedule(ReactSchedule);
    let elapsed = start.elapsed();

    let passes = world
        .get_resource::<FlushMetrics>()
        .map(|m| m.passes)
        .unwrap_or(0);

    if passes > 0 {
        log::debug!("signal2 ReactSchedule settled in {elapsed:?} over {passes} pass(es)");
    }
}
