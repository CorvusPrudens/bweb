//! A push-based reactive signal graph over `bevy_ecs`.
//!
//! Modules:
//! - [`graph`] — the engine: world-resident node state and the flush
//!   (mark → topological settle → bounded fixpoint).
//! - [`handle`] — signal handles ([`DerivedSignal`], [`ObserverSignal`]) and the
//!   fallible read API ([`SignalRead`]).
//! - [`signal`] — the [`Signal`] constructors (`signal` / `derive` / `memo`).
//! - [`insert`] — reactive Bundle insertion ([`SignalMap`], [`SignalOption`]).
//! - [`error`], [`reactive_context`] — the read error type and the
//!   thread-local source-collection used during evaluation.

use bevy_app::prelude::*;

mod error;
mod graph;
mod handle;
mod insert;
mod reactive_context;
mod signal;

#[cfg(test)]
mod tests;

use bevy_ecs::schedule::{IntoScheduleConfigs, SystemSet};
use graph::{ChangedNodes, PendingDirty, flush};

pub use error::{SignalError, SignalReadGuard, SignalResult};
pub use graph::{NodeStatus, SignalSystem, Sources, Subscribers};
pub use handle::{DerivedSignal, ObserverSignal, SignalRead, WatchBundle};
pub use insert::{MappedSignal, OptionSignal, ReactiveInsert, SignalMap, SignalOption};
pub use signal::Signal;

#[derive(SystemSet, PartialEq, Eq, Clone, Debug, Hash)]
pub enum ReactiveSystems {
    EvaluateGraph,
}

/// Wires up the signal2 reactive runtime.
pub struct Signal2Plugin;

impl Plugin for Signal2Plugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingDirty>()
            .init_resource::<ChangedNodes>()
            .add_systems(PostUpdate, flush.in_set(ReactiveSystems::EvaluateGraph));
    }
}
