//! Change-scanning input sources for components that mutate *in place*.
//!
//! Query observers (the [`signal`](super::SignalExt::signal) input) only fire on
//! component lifecycle events. In-place `&mut` mutations — the common case for
//! relationship collections like `Children` (add/remove a child among existing
//! ones, or reorder) — bump a change tick but emit no lifecycle event, so an
//! observer never sees them. [`Track::track`] fills that gap with a shared
//! `Changed<T>` scanner plus an `On<Remove, T>` observer:
//!
//! - the **scanner** (`scan_changed::<T>`, one per component type) runs each
//!   frame in [`ReactSchedule`](super::ReactSchedule)'s `Scan` set, delivers the
//!   new value to every node watching a changed entity, and pushes those nodes
//!   into [`PendingDirty`] so they settle in the same `Settle` pass;
//! - the **removal observer** (`on_remove_tracked::<T>`) covers the case the
//!   scanner is structurally blind to — `T` being removed outright (bevy drops
//!   `Children` when the last child leaves).
//!
//! Both feed the per-node writer, which takes `Option<&T>`: the scanner passes
//! `Some`, the observer passes `None`, and the extractor decides how absence
//! reads (for `Children`, an empty `Vec`).
//!
//! The machinery is registered **on demand**: the first `track` over a given
//! component type bootstraps its resource, scanner, and observer (idempotent,
//! keyed by [`ComponentId`]). The watched entity is bound through the same
//! deferred [`watch_entity`](TrackedSignal::watch_entity) /
//! [`watch_bundle`](TrackedSignal::watch_bundle) mechanism as
//! [`ObserverSignal`](super::ObserverSignal).

use bevy_ecs::{
    component::{ComponentId, Mutable, StorageType},
    lifecycle::{ComponentHook, HookContext},
    prelude::*,
    schedule::{IntoScheduleConfigs, Schedules},
    world::DeferredWorld,
};
use bevy_platform::collections::{HashMap, HashSet};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, RwLock};

use super::error::{SignalReadGuard, SignalResult};
use super::gc::SignalGc;
use super::graph::{NodeStatus, PendingDirty, Sources, Subscribers};
use super::handle::{SignalInner, SignalRead, WatchTarget, read_value};
use super::{ReactSchedule, ReactiveSystems};

/// A handle onto a change-scanning input signal produced by [`Track::track`].
pub struct TrackedSignal<O> {
    inner: SignalInner,
    shared: Arc<TrackShared<O>>,
}

// Hand-written so cloning a handle never requires `O: Clone` — the only `O`
// lives behind an `Arc`.
impl<O> Clone for TrackedSignal<O> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            shared: self.shared.clone(),
        }
    }
}

impl<O: Send + Sync + 'static> SignalRead for TrackedSignal<O> {
    type Value = O;

    fn read(&self) -> SignalResult<SignalReadGuard<'_, O>> {
        read_value(self.inner.entity, &self.shared.value)
    }
}

impl<O: Send + Sync + 'static> TrackedSignal<O> {
    /// Watch a specific, already-known entity.
    pub fn watch_entity(self, entity: Entity) -> Self {
        *self.shared.watch.lock().unwrap() = WatchTarget::Entity(entity);
        self
    }

    /// Watch the entity this signal is inserted into. Returns a
    /// [`TrackWatchBundle`] component: drop it into the target entity's bundle
    /// and the tracker binds to that entity (useful when the entity doesn't
    /// exist yet at signal creation).
    pub fn watch_bundle(&self) -> TrackWatchBundle<O> {
        *self.shared.watch.lock().unwrap() = WatchTarget::Bundle;
        TrackWatchBundle {
            shared: self.shared.clone(),
        }
    }
}

/// State shared between a [`TrackedSignal`]'s clones: its value cell, the pending
/// watch target, and the one-shot binder.
struct TrackShared<O> {
    value: RwLock<Option<O>>,
    watch: Mutex<WatchTarget>,
    builder: Mutex<Option<TrackBuilder>>,
}

/// Binds the tracker to a resolved entity: bootstraps the type's machinery,
/// registers the node's writer, and seeds the initial value. Consumed once.
type TrackBuilder = Box<dyn FnOnce(&mut World, Option<Entity>) + Send + Sync>;

/// Runs a [`TrackShared`]'s one-shot binder with the resolved entity. A no-op if
/// it was already consumed.
fn build_track<O>(shared: &TrackShared<O>, world: &mut World, watched: Option<Entity>) {
    if let Some(builder) = shared.builder.lock().unwrap().take() {
        builder(world, watched);
    }
}

/// Binds a [`TrackedSignal`] to the entity it is inserted into. Produced by
/// [`TrackedSignal::watch_bundle`].
pub struct TrackWatchBundle<O> {
    shared: Arc<TrackShared<O>>,
}

impl<O: Send + Sync + 'static> Component for TrackWatchBundle<O> {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;

    fn on_insert() -> Option<ComponentHook> {
        fn hook<O: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
            let host = ctx.entity;
            let shared = world
                .get::<TrackWatchBundle<O>>(host)
                .unwrap()
                .shared
                .clone();
            world
                .commands()
                .queue(move |world: &mut World| build_track(&shared, world, Some(host)));
        }
        Some(hook::<O>)
    }
}

/// A node watching one entity's `T`, plus the writer that delivers `Option<&T>`
/// into the node's value cell.
pub(crate) struct TrackedNode<T> {
    node: Entity,
    write: Box<dyn Fn(Option<&T>) + Send + Sync>,
}

/// Per-component-type registry: which nodes watch each entity's `T`.
pub(crate) struct TrackedSources<T>(pub(crate) HashMap<Entity, Vec<TrackedNode<T>>>);

impl<T> Default for TrackedSources<T> {
    fn default() -> Self {
        Self(HashMap::default())
    }
}

// `T` appears only behind `&T` in the writer's `dyn Fn`, so the registry is
// `Send + Sync` for any `T: 'static`.
impl<T: Send + Sync + 'static> Resource for TrackedSources<T> {}

/// Component types whose scanner + removal observer are already registered.
#[derive(Resource, Default)]
pub(crate) struct TrackedTypes(pub(crate) HashSet<ComponentId>);

/// Per-node GC teardown marker for a `track` source. Inserted (in the builder) once the
/// watched entity is known; its `on_remove` hook purges this node's [`TrackedSources`]
/// entry when the node is garbage-collected, dropping the last off-node `TrackShared`
/// clone. `PhantomData<fn() -> T>` keeps the marker `Send + Sync` for any `T`.
struct TrackGc<T> {
    watched: Entity,
    _t: PhantomData<fn() -> T>,
}

impl<T: Component> Component for TrackGc<T> {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;

    fn on_remove() -> Option<ComponentHook> {
        fn hook<T: Component>(mut world: DeferredWorld, ctx: HookContext) {
            let node = ctx.entity;
            let watched = world.get::<TrackGc<T>>(node).unwrap().watched;
            world.commands().queue(move |world: &mut World| {
                if let Some(mut sources) = world.get_resource_mut::<TrackedSources<T>>() {
                    if let Some(nodes) = sources.0.get_mut(&watched) {
                        nodes.retain(|tracked| tracked.node != node);
                        if nodes.is_empty() {
                            sources.0.remove(&watched);
                        }
                    }
                }
            });
        }
        Some(hook::<T>)
    }
}

/// Delivers each changed entity's new `T` to the nodes watching it and marks
/// them dirty. Runs in [`ReactiveSystems::Scan`], before the settle.
fn scan_changed<T: Component>(
    changed: Query<(Entity, &T), Changed<T>>,
    registry: Res<TrackedSources<T>>,
    mut pending: ResMut<PendingDirty>,
) {
    for (entity, comp) in &changed {
        if let Some(nodes) = registry.0.get(&entity) {
            for tracked in nodes {
                (tracked.write)(Some(comp));
                pending.0.push(tracked.node);
            }
        }
    }
}

/// Covers `T` being removed outright (which `Changed<T>` can't see): delivers the
/// absent value (`None`) to the watching nodes and marks them dirty.
fn on_remove_tracked<T: Component>(
    remove: On<Remove, T>,
    registry: Res<TrackedSources<T>>,
    mut pending: ResMut<PendingDirty>,
) {
    let entity = remove.event_target();
    if let Some(nodes) = registry.0.get(&entity) {
        for tracked in nodes {
            (tracked.write)(None);
            pending.0.push(tracked.node);
        }
    }
}

/// Idempotently registers `T`'s machinery on first use: its registry resource,
/// the `Changed<T>` scanner (in [`ReactSchedule`]'s `Scan` set), and the
/// `On<Remove, T>` observer. Keyed by [`ComponentId`].
fn bootstrap<T: Component>(world: &mut World) {
    let id = world.register_component::<T>();
    if !world.resource_mut::<TrackedTypes>().0.insert(id) {
        return;
    }
    world.init_resource::<TrackedSources<T>>();
    world.add_observer(on_remove_tracked::<T>);
    world
        .resource_mut::<Schedules>()
        .get_mut(ReactSchedule)
        .expect("ReactSchedule is initialised by Signal2Plugin")
        .add_systems(scan_changed::<T>.in_set(ReactiveSystems::Scan));
}

/// The `track` constructor, added to `Commands`.
pub trait Track {
    /// Creates an input signal driven by in-place mutations of a component `T` on
    /// the watched entity. `extract` maps the (possibly absent) component to the
    /// signal's value — `T` and `O` are inferred from an annotated closure, e.g.
    /// `commands.track(|c: Option<&Children>| ...)`. Bind the watched entity with
    /// [`TrackedSignal::watch_entity`] / [`TrackedSignal::watch_bundle`]; an
    /// unbound tracker never fires.
    fn track<T, O, F>(&mut self, extract: F) -> TrackedSignal<O>
    where
        T: Component,
        O: Send + Sync + 'static,
        F: Fn(Option<&T>) -> O + Send + Sync + 'static;
}

impl Track for Commands<'_, '_> {
    fn track<T, O, F>(&mut self, extract: F) -> TrackedSignal<O>
    where
        T: Component,
        O: Send + Sync + 'static,
        F: Fn(Option<&T>) -> O + Send + Sync + 'static,
    {
        let node = self
            .spawn((
                Subscribers::default(),
                Sources::default(),
                NodeStatus::Clean,
            ))
            .id();
        let shared = Arc::new(TrackShared {
            value: RwLock::new(None),
            watch: Mutex::new(WatchTarget::Global),
            builder: Mutex::new(None),
        });
        self.entity(node).insert(SignalGc::new(&shared, 2));
        let extract = Arc::new(extract);

        // Deferred: once the watched entity is known, bootstrap the type, seed the
        // initial value, and register the node's writer. Runs in the same command
        // as the seed, so no change slips between seeding and the scanner going
        // live.
        *shared.builder.lock().unwrap() = Some(Box::new({
            let shared = shared.clone();
            let extract = extract.clone();
            move |world: &mut World, watched: Option<Entity>| {
                let Some(entity) = watched else {
                    return;
                };
                bootstrap::<T>(world);
                let seeded = extract(world.get::<T>(entity));
                *shared.value.write().unwrap() = Some(seeded);
                let write: Box<dyn Fn(Option<&T>) + Send + Sync> = Box::new({
                    let shared = shared.clone();
                    let extract = extract.clone();
                    move |t: Option<&T>| {
                        *shared.value.write().unwrap() = Some(extract(t));
                    }
                });
                world
                    .resource_mut::<TrackedSources<T>>()
                    .0
                    .entry(entity)
                    .or_default()
                    .push(TrackedNode { node, write });
                // GC teardown: when this node is collected, its `on_remove` purges the
                // registry entry above (dropping the last off-node `shared` clone).
                world.entity_mut(node).insert(TrackGc::<T> {
                    watched: entity,
                    _t: PhantomData,
                });
            }
        }));

        // Finalize an `Entity` target now; a `Bundle` target defers to
        // `TrackWatchBundle`. An unbound (`Global`) tracker drops its binder to
        // break the `shared` <-> binder reference cycle.
        self.queue({
            let shared = shared.clone();
            move |world: &mut World| {
                enum Bind {
                    Entity(Entity),
                    Bundle,
                    Unbound,
                }
                let bind = match &*shared.watch.lock().unwrap() {
                    WatchTarget::Entity(e) => Bind::Entity(*e),
                    WatchTarget::Bundle => Bind::Bundle,
                    // Trackers have no dynamic-watch constructor, so `Dynamic`
                    // is unreachable here; treat it as unbound.
                    WatchTarget::Global | WatchTarget::Dynamic(_) => Bind::Unbound,
                };
                match bind {
                    Bind::Entity(entity) => build_track(&shared, world, Some(entity)),
                    Bind::Bundle => {}
                    Bind::Unbound => {
                        let _ = shared.builder.lock().unwrap().take();
                    }
                }
            }
        });

        TrackedSignal {
            inner: SignalInner { entity: node },
            shared,
        }
    }
}
