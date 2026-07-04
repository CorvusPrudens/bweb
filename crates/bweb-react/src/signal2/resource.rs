//! Change-tick-scanning input sources for **resources**.
//!
//! Resources emit no lifecycle events, so neither query observers
//! ([`signal`](super::SignalExt::signal)) nor component trackers
//! ([`track`](super::Track::track)) can see them change.
//! [`TrackResource::track_resource`] fills that gap the same way `track` does
//! for in-place component mutation: a shared per-type **scanner** runs each
//! frame in [`ReactSchedule`](super::ReactSchedule)'s `Scan` set, and when the
//! resource's change tick moves it delivers a clone to every registered node
//! and pushes them into [`PendingDirty`]. One `is_changed` check per type per
//! frame — no per-node polling.
//!
//! The machinery is registered **on demand**: the first `track_resource` of a
//! given type bootstraps its registry and scanner (idempotent, keyed by
//! [`ComponentId`]). Nodes are garbage-collected like any other signal; a
//! per-node hook purges the registry entry on collection.

use bevy_ecs::{
    component::{ComponentId, Mutable, StorageType},
    lifecycle::{ComponentHook, HookContext},
    prelude::*,
    schedule::IntoScheduleConfigs,
    world::DeferredWorld,
};
use bevy_platform::collections::HashSet;
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};

use super::gc::SignalGc;
use super::graph::{NodeStatus, PendingDirty, Sources, Subscribers};
use super::handle::{DerivedSignal, SignalInner};
use super::ReactiveSystems;

/// A node watching resource `R`, plus the writer that delivers a fresh clone
/// into the node's value cell.
struct ResourceNode<R> {
    node: Entity,
    write: Box<dyn Fn(&R) + Send + Sync>,
}

/// Per-resource-type registry of watching nodes.
struct TrackedResourceNodes<R>(Vec<ResourceNode<R>>);

impl<R> Default for TrackedResourceNodes<R> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

// `R` appears only behind `&R` in the writer's `dyn Fn`, so the registry is
// `Send + Sync` for any `R: 'static`.
impl<R: Send + Sync + 'static> Resource for TrackedResourceNodes<R> {}

/// Resource types whose scanner is already registered.
#[derive(Resource, Default)]
struct TrackedResourceTypes(HashSet<ComponentId>);

/// Per-node GC teardown marker: purges this node's [`TrackedResourceNodes`]
/// entry when the node is collected, dropping the off-node value-cell clone.
struct ResourceTrackGc<R> {
    _r: PhantomData<fn() -> R>,
}

impl<R: Resource> Component for ResourceTrackGc<R> {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;

    fn on_remove() -> Option<ComponentHook> {
        fn hook<R: Resource>(mut world: DeferredWorld, ctx: HookContext) {
            let node = ctx.entity;
            world.commands().queue(move |world: &mut World| {
                if let Some(mut nodes) = world.get_resource_mut::<TrackedResourceNodes<R>>() {
                    nodes.0.retain(|tracked| tracked.node != node);
                }
            });
        }
        Some(hook::<R>)
    }
}

/// Delivers the changed resource to every watching node and marks them dirty.
/// `is_changed` is relative to this system's own last run, so one scanner
/// serves every node of the type. Runs in [`ReactiveSystems::Scan`].
fn scan_resource<R: Resource + Clone>(
    resource: Option<Res<R>>,
    registry: Res<TrackedResourceNodes<R>>,
    mut pending: ResMut<PendingDirty>,
) {
    let Some(resource) = resource else {
        return;
    };
    if !resource.is_changed() {
        return;
    }
    for tracked in &registry.0 {
        (tracked.write)(&resource);
        pending.0.push(tracked.node);
    }
}

/// Idempotently registers `R`'s machinery on first use: its registry resource
/// and the change-tick scanner. Keyed by [`ComponentId`].
fn bootstrap<R: Resource + Clone>(world: &mut World) {
    let id = world.components_registrator().register_resource::<R>();
    if !world
        .get_resource_or_init::<TrackedResourceTypes>()
        .0
        .insert(id)
    {
        return;
    }
    world.init_resource::<TrackedResourceNodes<R>>();
    super::register_scanner(world, |schedule| {
        schedule.add_systems(scan_resource::<R>.in_set(ReactiveSystems::Scan));
    });
}

/// The `track_resource` constructor, added to `Commands`.
pub trait TrackResource {
    /// Creates an input signal driven by change ticks of resource `R`. The
    /// value is a clone of the resource, refreshed whenever its tick moves;
    /// it reads as `NotReady` until `R` first exists. Propagation is not
    /// value-gated — gate downstream with a [`memo`](super::SignalExt::memo)
    /// when consumers should only react to a relevant slice of `R`.
    fn track_resource<R: Resource + Clone>(&mut self) -> DerivedSignal<R>;
}

impl TrackResource for Commands<'_, '_> {
    fn track_resource<R: Resource + Clone>(&mut self) -> DerivedSignal<R> {
        let node = self
            .spawn((
                Subscribers::default(),
                Sources::default(),
                NodeStatus::Clean,
            ))
            .id();
        let value = Arc::new(RwLock::new(None));
        self.entity(node).insert(SignalGc::new(&value, 2));

        // Bootstrap, seed, and register in one command so no change slips
        // between the seed and the scanner going live.
        self.queue({
            let value = value.clone();
            move |world: &mut World| {
                bootstrap::<R>(world);
                if let Some(resource) = world.get_resource::<R>() {
                    *value.write().unwrap() = Some(resource.clone());
                    // Wake subscribers that read `NotReady` before this seed
                    // (e.g. deferred sinks with a pre-registered edge).
                    world.resource_mut::<PendingDirty>().0.push(node);
                }
                let write: Box<dyn Fn(&R) + Send + Sync> = Box::new({
                    let value = value.clone();
                    move |resource: &R| {
                        *value.write().unwrap() = Some(resource.clone());
                    }
                });
                world
                    .resource_mut::<TrackedResourceNodes<R>>()
                    .0
                    .push(ResourceNode { node, write });
                // GC teardown: collecting the node purges its registry entry.
                world
                    .entity_mut(node)
                    .insert(ResourceTrackGc::<R> { _r: PhantomData });
            }
        });

        DerivedSignal {
            inner: SignalInner { entity: node },
            value,
        }
    }
}
