//! Signals over resources.
//!
//! A resource has no entity, so it has no `SubscriberSet` to join and nothing
//! for the scan to visit. What it does have is a change tick, and one
//! `is_changed` check per type per pass is enough to drive everything else.
//!
//! So a resource signal is a [`Cell`] with a scanner behind it. The first
//! signal over `R` registers one scanner system for the type; every pass it
//! asks whether `R`'s tick moved, and if it did it pushes a fresh value into
//! each watching cell. From there the cell tier does the rest — the write
//! flags the cell, [`drain_dirty_cells`] marks its readers, and a derived node
//! or an effect that read the signal re-runs. Nothing here is a new kind of
//! node.
//!
//! # The value is copied out
//!
//! A cell's value lives in its handle, so the scanner has to put something
//! there, and that means [`resource`] wants `R: Clone`. For a resource that is
//! large or changes often, clone the part you need instead:
//!
//! ```ignore
//! // Copies the whole registry every time anything in it changes.
//! let protos = commands.resource::<RegisteredPrototypes>();
//!
//! // Copies one entity.
//! let knob = commands.resource_with(|p: &RegisteredPrototypes| p.get("knob").copied());
//! ```
//!
//! The projection also narrows what the *scan* costs, but not what wakes it:
//! propagation is driven by `R`'s tick, not by whether the projection's output
//! moved. Put a [`derive`] in front when readers should only see real changes.
//!
//! # Lifetime
//!
//! The cell is owned by the [`ResSignal`] handle, exactly as
//! [`Cell`](super::cell::Cell) is by its own — the scanner's registry holds
//! only a weak handle and prunes its entry when the last real one goes. So a
//! signal has to be kept (captured by the closure that reads it, usually) for
//! as long as anything reads it.
//!
//! [`drain_dirty_cells`]: super::cell::drain_dirty_cells
//! [`derive`]: super::SignalExt::derive
//! [`resource`]: super::SignalExt::resource

use core::any::TypeId;

use bevy_ecs::{prelude::*, system::BoxedSystem};
use bevy_platform::collections::HashSet;

use crate::signal2::{
    cell::{Cell, CellRef},
    dynamic::Signal,
    source::SignalStore,
};

/// A signal over a resource, or over some part of one.
///
/// Built with [`resource`](super::SignalExt::resource) or
/// [`resource_with`](super::SignalExt::resource_with). Read it with
/// [`read`](Self::read) or [`get`](Self::get) from inside a derived closure or
/// an effect, which subscribes that node to it, and with [`peek`](Self::peek)
/// from anywhere else.
///
/// The value is an `Option` because a resource need not exist: a signal built
/// before its resource is inserted reads `None` until it is.
pub struct ResSignal<T>(Cell<Option<T>>);

impl<T> Clone for ResSignal<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Send + Sync + 'static> ResSignal<T> {
    /// The entity standing in for this signal, which its readers subscribe to.
    pub fn entity(&self) -> Entity {
        self.0.entity()
    }

    /// Read the value, subscribing the node currently being evaluated.
    pub fn read(&self, store: &SignalStore) -> CellRef<'_, Option<T>> {
        self.0.read(store)
    }

    /// [`read`](Self::read) without the subscription, for event handlers and
    /// anything else outside the graph.
    pub fn peek(&self) -> CellRef<'_, Option<T>> {
        self.0.peek()
    }

    /// The cell behind this signal, for a caller that wants the [`Cell`] API.
    ///
    /// Writing it is pointless — the next scan of the resource overwrites
    /// whatever was put there.
    pub fn cell(&self) -> Cell<Option<T>> {
        self.0.clone()
    }
}

impl<T: Clone + Send + Sync + 'static> ResSignal<T> {
    /// [`read`](Self::read), cloned, for a caller that would rather not hold
    /// the cell's read guard.
    pub fn get(&self, store: &SignalStore) -> Option<T> {
        self.0.get(store)
    }

    /// [`get`](Self::get) without the subscription.
    pub fn peek_cloned(&self) -> Option<T> {
        self.0.peek_cloned()
    }
}

impl<T: Send + Sync + 'static> From<ResSignal<T>> for Signal<Option<T>> {
    fn from(signal: ResSignal<T>) -> Self {
        signal.0.into()
    }
}

/// The cells watching one resource type, and how to fill each.
///
/// `R` appears only behind `&R` in the writers, so this is `Send + Sync`
/// whatever `R` is — though `R: Resource` already guarantees that.
struct WatchingCells<R> {
    /// Each writer holds a weak handle on its cell and reports whether that
    /// handle is still live, which is how a collected cell's entry is pruned.
    writers: Vec<CellWriter<R>>,
}

/// Fills one cell from the resource, reporting whether the cell is still there.
type CellWriter<R> = Box<dyn Fn(&R) -> bool + Send + Sync>;

impl<R> Default for WatchingCells<R> {
    fn default() -> Self {
        Self {
            writers: Vec::new(),
        }
    }
}

impl<R: Send + Sync + 'static> Resource for WatchingCells<R> {}

/// Resource types whose scanner is already registered, and the scanners
/// themselves.
#[derive(Resource, Default)]
pub(crate) struct ResourceScanners {
    registered: HashSet<TypeId>,
    /// Registered but not yet initialized — a scanner is built from a
    /// `Commands` queue, which has no `&mut World` to initialize it with.
    fresh: Vec<BoxedSystem<(), ()>>,
    live: Vec<BoxedSystem<(), ()>>,
}

impl ResourceScanners {
    fn is_idle(&self) -> bool {
        self.fresh.is_empty() && self.live.is_empty()
    }
}

/// Push a fresh value into every cell watching `R`, if `R` changed.
///
/// `is_changed` is relative to this system's own last run, and the system runs
/// once per settle pass — so a resource written during `Update` wakes readers
/// on the frame's first pass and stays quiet for the rest of it. A scanner that
/// has never run reads as changed, which is what seeds a signal built before
/// its resource existed.
fn scan_resource<R: Resource>(resource: Option<Res<R>>, mut watching: ResMut<WatchingCells<R>>) {
    let Some(resource) = resource else {
        return;
    };
    if !resource.is_changed() {
        return;
    }

    // Nothing watches the registry, so a change tick on it would be noise.
    watching
        .bypass_change_detection()
        .writers
        .retain(|write| write(&resource));
}

/// Run every resource scanner, initializing any registered since the last pass.
///
/// Called at the top of a settle pass, before
/// [`drain_dirty_cells`](super::cell::drain_dirty_cells), so that a resource
/// change and the cell write it produces land in the same pass.
pub(crate) fn run_resource_scanners(world: &mut World) {
    if world.resource::<ResourceScanners>().is_idle() {
        return;
    }

    world.resource_scope(|world, mut scanners: Mut<ResourceScanners>| {
        let scanners = &mut *scanners;

        // Taken rather than drained in place so the initialized systems can be
        // pushed onto `live` without holding two borrows of `scanners`.
        for mut system in core::mem::take(&mut scanners.fresh) {
            system.initialize(world);
            scanners.live.push(system);
        }

        for system in &mut scanners.live {
            if let Err(e) = system.run((), world) {
                log::error!("signal2: a resource scanner failed to run: {e}");
            }
        }
    });
}

/// Build a signal over some part of resource `R`.
///
/// The whole of [`SignalExt::resource_with`](super::SignalExt::resource_with);
/// [`resource`](super::SignalExt::resource) is this with `R::clone`.
pub(crate) fn resource_with<R, T, F>(commands: &mut Commands, project: F) -> ResSignal<T>
where
    R: Resource,
    T: Send + Sync + 'static,
    F: Fn(&R) -> T + Send + Sync + 'static,
{
    let cell = Cell::new(commands, None);
    let weak = cell.downgrade();

    // Bootstrap, seed, and register in one command, so no change can slip in
    // between the seed and the scanner going live.
    commands.queue(move |world: &mut World| {
        bootstrap::<R>(world);

        // A scanner that has never run would seed this itself on the next
        // pass. Doing it here as well is what lets a derived node built in the
        // same breath read a value immediately rather than a pass later.
        if let Some(resource) = world.get_resource::<R>()
            && let Some(cell) = weak.upgrade()
        {
            cell.set(Some(project(resource)));
        }

        let write = {
            let weak = weak.clone();
            move |resource: &R| match weak.upgrade() {
                Some(cell) => {
                    cell.set(Some(project(resource)));
                    true
                }
                None => false,
            }
        };

        world
            .get_resource_or_init::<WatchingCells<R>>()
            .writers
            .push(Box::new(write));
    });

    ResSignal(cell)
}

/// How many live cells the scanner is writing for `R`.
#[cfg(test)]
pub(crate) fn watching_count<R: Resource>(world: &World) -> usize {
    world
        .get_resource::<WatchingCells<R>>()
        .map_or(0, |watching| watching.writers.len())
}

/// Idempotently register `R`'s scanner and its registry.
fn bootstrap<R: Resource>(world: &mut World) {
    let mut scanners = world.get_resource_or_init::<ResourceScanners>();
    if !scanners.registered.insert(TypeId::of::<R>()) {
        return;
    }
    scanners
        .fresh
        .push(Box::new(IntoSystem::into_system(scan_resource::<R>)));
}
