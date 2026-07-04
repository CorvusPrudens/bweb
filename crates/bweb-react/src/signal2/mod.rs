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
mod resource;
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
use graph::{ChangedNodes, FlushWork, PendingDirty, flush};
use track::TrackedTypes;

pub use error::{SignalError, SignalReadGuard, SignalResult};
pub use gc::SweepFrequency;
pub use graph::{NodeStatus, SignalSystem, Sources, Subscribers};
pub use handle::{DerivedSignal, ObserverSignal, Signal, SignalRead, WatchBundle};
pub use insert::{MappedSignal, OptionSignal, ReactiveInsert, SignalMap, SignalOption};
pub use list::{ReactiveList, ReactiveListExt};
pub use resource::TrackResource;
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

/// Scanner registrations that arrived while [`ReactSchedule`] was executing.
///
/// `run_schedule` takes the schedule out of the `Schedules` registry for the
/// duration of the run, so a `track`/`track_resource` bootstrap reached from a
/// node evaluated *inside* the flush cannot add its scanner system directly.
/// [`register_scanner`] parks the registration here; [`run_react_schedule`]
/// drains it around each run.
#[derive(Resource, Default)]
pub(crate) struct PendingScanners(Vec<Box<dyn FnOnce(&mut World) + Send + Sync>>);

/// Adds a scanner system to [`ReactSchedule`], deferring the registration when
/// the schedule is currently executing. The caller must have seeded the
/// signal's value itself (and pushed [`PendingDirty`]) — a deferred scanner
/// only misses in-place mutations that land before the next
/// [`run_react_schedule`], which then scans them normally.
pub(crate) fn register_scanner(
    world: &mut World,
    add: impl FnOnce(&mut bevy_ecs::schedule::Schedule) + Send + Sync + 'static,
) {
    let mut schedules = world.resource_mut::<bevy_ecs::schedule::Schedules>();
    if let Some(schedule) = schedules.get_mut(ReactSchedule) {
        add(schedule);
    } else {
        world
            .resource_mut::<PendingScanners>()
            .0
            .push(Box::new(move |world: &mut World| {
                let mut schedules = world.resource_mut::<bevy_ecs::schedule::Schedules>();
                let schedule = schedules
                    .get_mut(ReactSchedule)
                    .expect("ReactSchedule returns to the registry after its run");
                add(schedule);
            }));
    }
}

/// Applies scanner registrations parked while the schedule was running.
fn drain_pending_scanners(world: &mut World) {
    loop {
        let pending = core::mem::take(&mut world.resource_mut::<PendingScanners>().0);
        if pending.is_empty() {
            break;
        }
        for register in pending {
            register(world);
        }
    }
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
            .init_resource::<PendingScanners>()
            .init_resource::<FlushWork>()
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

/// Upper bound on schedule re-runs per frame; a converging cascade breaks out
/// as soon as a flush does no work.
const MAX_SCHEDULE_RUNS: usize = 8;

/// Runs [`ReactSchedule`] until quiescent (bounded). The flush iterates to a
/// fixpoint internally, but **scanners** run only once per schedule run — and
/// sinks evaluated inside the flush can mutate tracked components *after* this
/// run's scan. Without a follow-up run those changes would sit unseen until
/// the next external event, which may never arrive (idle/headless). Re-running
/// while the flush reports work settles every cascade within the frame; idle
/// frames still cost a single run.
#[cfg(not(feature = "dev"))]
fn run_react_schedule(world: &mut World) {
    drain_pending_scanners(world);
    for _ in 0..MAX_SCHEDULE_RUNS {
        world.run_schedule(ReactSchedule);
        drain_pending_scanners(world);
        if !world.resource::<FlushWork>().0 {
            break;
        }
    }
}

/// `dev` build: times the whole [`ReactSchedule`] run (change scanners + graph
/// settle) and reports the elapsed wall time alongside the fixpoint pass count
/// published by [`flush`]. Only logs on frames that did work, so idle frames stay
/// quiet.
#[cfg(feature = "dev")]
fn run_react_schedule(world: &mut World) {
    drain_pending_scanners(world);
    let start = bevy_platform::time::Instant::now();
    let mut passes = 0usize;
    let mut runs = 0usize;
    for _ in 0..MAX_SCHEDULE_RUNS {
        world.run_schedule(ReactSchedule);
        drain_pending_scanners(world);
        runs += 1;
        passes += world
            .get_resource::<FlushMetrics>()
            .map(|m| m.passes)
            .unwrap_or(0);
        if !world.resource::<FlushWork>().0 {
            break;
        }
    }
    let elapsed = start.elapsed();

    if passes > 0 {
        log::debug!(
            "signal2 ReactSchedule settled in {elapsed:?} over {passes} pass(es), {runs} run(s)"
        );
    }
}
