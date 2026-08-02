//! Reactive values that live in the handle rather than in a component.
//!
//! A cell splits the two halves apart. The value lives in an `Arc` behind
//! an `RwLock`. The identity stays an entity, allowing it to resuse the
//! normal change-tick driven code path.
//!
//! # Waking readers
//!
//! A write has no world access, so it cannot mark its subscribers directly.
//! Instead it sets a flag on its own `Arc` and bumps a process-wide counter,
//! and [`drain_dirty_cells`] — one step at the top of each settle pass — turns
//! flagged cells into dirty nodes. The counter is what keeps that from being a
//! poll in the steady state: a pass in which no cell anywhere was written costs
//! a single relaxed load and touches the registry not at all.
//!
//! [`SignalReads`]: super::source::SignalReads
//! [`NodeSource`]: super::derived::NodeSource
//! [`UnsubscribeFn`]: super::source::UnsubscribeFn

use core::ops::{Deref, DerefMut};
use std::sync::{
    Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use bevy_ecs::prelude::*;
use smallvec::SmallVec;

use crate::signal2::{
    ReactiveWork,
    derived::{NodeSource, NodeSources, NodeSubscribers, raise_level},
    source::SignalStore,
};

/// Total cell writes across the process.
///
/// Multiple worlds may cause additional minor work for each other
/// when writing to this value.
static CELL_WRITES: AtomicU64 = AtomicU64::new(0);

/// The non-generic half of a cell, so [`CellRegistry`] can hold cells of every
/// value type in one list.
trait CellFlag: Send + Sync {
    /// Clear the dirty flag, reporting whether it was set.
    fn take_dirty(&self) -> bool;
}

struct CellInner<T> {
    value: RwLock<T>,
    dirty: AtomicBool,
}

impl<T: Send + Sync> CellFlag for CellInner<T> {
    fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }
}

/// A signal whose value lives in the handle.
///
/// Build one with [`SignalExt::cell`](super::SignalExt::cell). Read it with
/// [`read`](Cell::read) from inside a derived closure or an effect, which
/// subscribes that node to the cell, and with [`peek`](Cell::peek) from
/// anywhere else. Write it with [`set`](Cell::set), [`update`](Cell::update),
/// or [`write`](Cell::write) from anywhere at all.
///
/// The cell's entity is despawned once the last handle is dropped.
pub struct Cell<T> {
    entity: Entity,
    inner: Arc<CellInner<T>>,
}

impl<T> Clone for Cell<T> {
    fn clone(&self) -> Self {
        Self {
            entity: self.entity,
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: Send + Sync + 'static> Cell<T> {
    pub(crate) fn new(commands: &mut Commands, value: T) -> Self {
        // Spawned with the subscriber list already on it so that
        // `subscribe_cell` never has to consider the entity half-built.
        let entity = commands.spawn(NodeSubscribers::default()).id();
        let inner = Arc::new(CellInner {
            value: RwLock::new(value),
            dirty: AtomicBool::new(false),
        });

        // Weak, so the registry is what notices the last handle going rather
        // than what keeps the cell alive.
        let flag: Weak<dyn CellFlag> = Arc::downgrade(&inner) as Weak<dyn CellFlag>;
        commands.queue(move |world: &mut World| {
            world
                .resource_mut::<CellRegistry>()
                .cells
                .push(CellEntry { node: entity, flag });
        });

        Self { entity, inner }
    }

    /// The entity standing in for this cell, which its readers subscribe to.
    pub fn entity(&self) -> Entity {
        self.entity
    }

    fn value(&self) -> RwLockReadGuard<'_, T> {
        // A poisoned cell means a reader or writer panicked while holding the
        // lock, which says nothing about the value itself — and a widget whose
        // state permanently stops being readable is worse than one carrying a
        // value written by a panicking closure, so the lock is recovered rather
        // than propagated.
        self.inner
            .value
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Read the cell, subscribing the node currently being evaluated to it.
    ///
    /// The counterpart of [`SourceSignal::get`](super::source::SourceSignal::get):
    /// call it from a derived closure or an effect body and that node re-runs
    /// whenever the cell is written. Read a cell any other way — including
    /// through a [`SignalStore`] borrowed by an ordinary system — and the
    /// recorded read is discarded, so prefer [`peek`](Cell::peek) there to say
    /// so.
    pub fn read(&self, store: &SignalStore) -> CellRef<'_, T> {
        store.record_at(self.entity, subscribe_cell);
        CellRef(self.value())
    }

    /// [`read`](Cell::read) without the subscription, and without needing a
    /// store at all — for event handlers and anything else outside the graph.
    pub fn peek(&self) -> CellRef<'_, T> {
        CellRef(self.value())
    }

    /// Take the cell's value for writing.
    ///
    /// The cell is marked dirty when the guard is first dereferenced mutably,
    /// not when it is taken, so a `write` that turns out to change nothing
    /// wakes nobody.
    pub fn write(&self) -> CellMut<'_, T> {
        CellMut {
            value: self
                .inner
                .value
                .write()
                .unwrap_or_else(PoisonError::into_inner),
            dirty: &self.inner.dirty,
        }
    }

    /// Replace the value.
    pub fn set(&self, value: T) {
        *self.write() = value;
    }

    /// Change the value in place.
    pub fn update<F: FnOnce(&mut T)>(&self, f: F) {
        f(&mut *self.write());
    }
}

impl<T: Clone + Send + Sync + 'static> Cell<T> {
    /// [`read`](Cell::read), cloned, for a caller that would rather not hold
    /// the lock — which is most of them, since holding a read guard across a
    /// [`set`](Cell::set) on the same cell would deadlock.
    pub fn get(&self, store: &SignalStore) -> T {
        self.read(store).clone()
    }

    /// [`get`](Cell::get) without the subscription.
    pub fn peek_cloned(&self) -> T {
        self.peek().clone()
    }
}

pub struct CellRef<'a, T>(RwLockReadGuard<'a, T>);

impl<T> Deref for CellRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct CellMut<'a, T> {
    value: RwLockWriteGuard<'a, T>,
    dirty: &'a AtomicBool,
}

impl<T> Deref for CellMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for CellMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.dirty.store(true, Ordering::Relaxed);
        CELL_WRITES.fetch_add(1, Ordering::Relaxed);
        &mut self.value
    }
}

/// Subscribe `node` to the cell standing on `cell`.
///
/// The cell counterpart of
/// [`subscribe_derived`](super::source::subscribe_derived), recorded as a plain
/// [`SubscribeFn`](super::source::SubscribeFn) so the derived pass can wire the
/// edge without knowing it is a cell.
fn subscribe_cell(world: &mut World, cell: Entity, node: Entity) {
    // A cell's value is in its handle, so nothing is ever upstream of it: it is
    // level zero unconditionally, where a component source has to be asked. The
    // raise still walks downstream, so subscribers wired before this edge
    // existed deepen with it.
    raise_level(world, node, 1);

    if world
        .get::<NodeSources>(node)
        .is_some_and(|sources| sources.contains(cell))
    {
        return;
    }

    // No `PendingNodes` seed here, unlike `subscribe_derived`. A brand new edge
    // to a component source can have read a value that is still sitting in the
    // command queue, and the node has to re-run to pay that debt. A cell has no
    // such debt: a write goes straight into the handle, so the read that
    // reported this edge already saw the current value.

    {
        let Ok(mut cell_entity) = world.get_entity_mut(cell) else {
            // The cell was despawned — its last handle went — between the read
            // that reported it and this wiring.
            return;
        };
        let mut subscribers = cell_entity
            .entry::<NodeSubscribers>()
            .or_default()
            .into_mut();
        // No shared-mark check, unlike the component path: two distinct signal
        // entities can point at one target and have to share a mark, but a cell
        // is its own target, so the `contains` above has already settled it.
        subscribers.0.push(node);
    }

    if let Some(mut sources) = world.get_mut::<NodeSources>(node) {
        sources.0.push(NodeSource {
            signal: cell,
            target: cell,
            unsubscribe: unsubscribe_cell,
        });
    }
}

/// Drop `node` from the cell's subscriber list.
fn unsubscribe_cell(world: &mut World, cell: Entity, node: Entity) {
    let Ok(mut cell_entity) = world.get_entity_mut(cell) else {
        return;
    };

    if let Some(mut subscribers) = cell_entity.get_mut::<NodeSubscribers>() {
        subscribers.0.retain(|subscriber| *subscriber != node);
    }
}

/// One live cell, as the world sees it.
struct CellEntry {
    /// The entity standing in for the cell.
    node: Entity,
    /// The cell's dirty flag. Weak: an expired handle is how the sweep learns
    /// the cell's last owner is gone and the entity can be despawned.
    flag: Weak<dyn CellFlag>,
}

/// Every cell that has been built and not yet collected.
#[derive(Resource, Default)]
pub(crate) struct CellRegistry {
    cells: Vec<CellEntry>,
    /// [`CELL_WRITES`] as of the last sweep.
    seen: u64,
}

/// Mark the readers of every cell written since the last pass, and collect the
/// cells whose last handle has gone.
///
/// Both halves are done in one walk because both are gated on the same thing:
/// if nothing anywhere has been written, no cell has changed *and* the walk is
/// not worth paying for. The cost of pairing them is that a cell dropped in a
/// frame where nothing is written keeps its entity until the next write —
/// which is one entity holding one empty `SmallVec`, and it goes on the next
/// interaction.
pub(crate) fn drain_dirty_cells(world: &mut World, work: &mut ReactiveWork) {
    let writes = CELL_WRITES.load(Ordering::Relaxed);
    if world.resource::<CellRegistry>().seen == writes {
        return;
    }

    world.resource_scope(|world, mut registry: Mut<CellRegistry>| {
        registry.seen = writes;

        let mut collected: SmallVec<[Entity; 4]> = SmallVec::new();
        registry.cells.retain(|entry| {
            let Some(flag) = entry.flag.upgrade() else {
                collected.push(entry.node);
                return false;
            };

            if flag.take_dirty()
                && let Some(subscribers) = world.get::<NodeSubscribers>(entry.node)
            {
                // Marking rather than evaluating, exactly as a component
                // source's subscription does, so a node reading two cells that
                // both changed still evaluates once.
                for &node in &subscribers.0 {
                    work.mark_dirty(node);
                }
            }

            true
        });

        for node in collected {
            world.despawn(node);
        }
    });
}
