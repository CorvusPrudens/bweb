use std::{any::TypeId, sync::Mutex};

use bevy_app::{Plugin, PostUpdate};
use bevy_ecs::{
    entity::EntityHashSet,
    lifecycle::HookContext,
    prelude::*,
    query::{
        QueryAccessError, QueryData, QueryEntityError, QueryFilter, ReadOnlyQueryData,
        ReleaseStateQueryData,
    },
    relationship::Relationship,
    system::{BoxedReadOnlySystem, InMut},
    world::DeferredWorld,
};
use bevy_platform::collections::{HashMap, HashSet};

pub mod cell;
pub mod derived;
pub mod dynamic;
pub mod effect;
pub mod list;
pub mod mapped;
pub mod source;

#[cfg(test)]
mod tests;

use smallvec::SmallVec;
use source::SourceSignal;

use crate::signal2::{
    cell::{Cell, CellRegistry, drain_dirty_cells},
    derived::{PendingNodes, run_derived_nodes},
    effect::run_pending_effects,
    list::{ListSource, ReactiveList},
    source::{SignalReads, SourceSignalView},
};

pub struct ReactivePlugin;

impl Plugin for ReactivePlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.init_resource::<ReactiveSystems>()
            .init_resource::<PendingNodes>()
            .init_resource::<CellRegistry>()
            .init_resource::<SharedSignalSystems>()
            // Before `InnerReactiveSystems`, whose `FromWorld` initializes a
            // system that reads it.
            .init_resource::<SignalReads>()
            .init_resource::<InnerReactiveSystems>()
            .add_systems(PostUpdate, run_reactive_schedule);
    }
}

pub trait SignalData: ReadOnlyQueryData + ReleaseStateQueryData + 'static {
    type Filter: QueryFilter;
}

impl<T: Component> SignalData for &'static T {
    type Filter = Changed<T>;
}

macro_rules! impl_signal_data {
    ($($ty:ident),*) => {
        impl<$($ty: SignalData),*> SignalData for ($($ty,)*) {
            type Filter = Or<($($ty::Filter,)*)>;
        }
    };
}

impl_signal_data!(A);
impl_signal_data!(A, B);
impl_signal_data!(A, B, C);
impl_signal_data!(A, B, C, D);

/// Scratch state threaded through one settle pass as the reactive systems'
/// [`InMut`] input.
///
/// It exists so a subscriber can report work without a `ResMut` — the signal
/// systems stay read-only, which keeps them cheap to run by hand and lets a
/// subscriber run nested read-only systems against the `&World` it is handed.
#[derive(Default)]
pub struct ReactiveWork {
    /// Derived nodes to re-evaluate before this pass ends.
    dirty: Vec<Entity>,
    /// Reused scratch for deduplicating `dirty`.
    seen: EntityHashSet,
    /// Effects marked this pass, held back for the exclusive step that runs
    /// them once the read-only passes are done.
    effects: Vec<Entity>,
    /// Subscriber closures dispatched this pass. Zero means the graph settled.
    dispatched: usize,
}

impl ReactiveWork {
    /// Queue a derived node for re-evaluation later in this pass.
    ///
    /// Duplicates are fine — the derived pass dedups, so a node with several
    /// changed inputs still evaluates once.
    pub fn mark_dirty(&mut self, node: Entity) {
        self.dirty.push(node);
    }

    /// Hand an effect to the exclusive step at the end of this pass.
    ///
    /// Called from the derived pass, which has already deduplicated and level
    /// sorted, so what arrives here is each effect once in dependency order.
    pub(crate) fn queue_effect(&mut self, node: Entity) {
        self.effects.push(node);
    }

    pub(crate) fn take_effects(&mut self) -> Vec<Entity> {
        core::mem::take(&mut self.effects)
    }

    /// Subscriber closures dispatched during the pass just run.
    pub fn dispatched(&self) -> usize {
        self.dispatched
    }
}

/// What a subscriber closure is handed when its source changes.
pub struct SubscriberContext<'a, 'w, 's> {
    /// Read-only view of the world. Present so a subscriber can run nested
    /// read-only systems (`map_system`) or read state outside its own `D`.
    pub world: &'a World,
    pub work: &'a mut ReactiveWork,
    pub commands: Commands<'w, 's>,
}

type SubscriberClosure<D> = Box<
    dyn for<'w, 's, 'a, 'cw, 'cs> Fn(
            <D as QueryData>::Item<'w, 's>,
            SubscriberContext<'a, 'cw, 'cs>,
        ) + Send
        + Sync,
>;

/// A reactive system: read-only, driven by a [`ReactiveWork`] scratch buffer.
pub type SignalSystem = BoxedReadOnlySystem<InMut<'static, ReactiveWork>, ()>;

pub trait IntoSignalSystem {
    fn into_signal_system() -> SignalSystem;
}

impl<D: SignalData> IntoSignalSystem for D
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    fn into_signal_system() -> SignalSystem {
        let system = |InMut(work): InMut<ReactiveWork>,
                      world: &World,
                      query: Query<(D, &SubscriberSet<D>), D::Filter>,
                      systems: Query<&RegisteredSignalSystem<D>>,
                      mut commands: Commands| {
            for (data, subs) in &query {
                for closure in &subs.closures.f {
                    work.dispatched += 1;
                    closure(
                        data,
                        SubscriberContext {
                            world,
                            work: &mut *work,
                            commands: commands.reborrow(),
                        },
                    );
                }

                // Unlike the closures, the systems' owner array is on the hot
                // path: a registered system is shared between every subscriber
                // that uses it, so the owner is the only thing saying which
                // entity this run's output belongs to.
                for (&system, sub) in subs.systems.f.iter().zip(&subs.systems.subs) {
                    let Ok(registered) = systems.get(system) else {
                        log::error!("signal2: mapper system entity {system} is missing");
                        continue;
                    };
                    let Ok(mut registered) = registered.system.lock() else {
                        log::error!("signal2: mapper system {system} panicked on an earlier run");
                        continue;
                    };

                    work.dispatched += 1;
                    // shrink the lifetime so we don't get borrows too long
                    let data = D::shrink(D::release_state(data));

                    if let Err(e) =
                        registered.run_readonly((sub.owner, data, commands.reborrow()), world)
                    {
                        log::error!("signal2: subscriber system failed to run: {e}");
                    }
                }
            }
        };

        Box::new(IntoSystem::into_system(system))
    }
}

#[derive(Component)]
#[component(on_add = Self::add)]
struct SubscriberSet<D: SignalData>
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    closures: NodeClosures<D>,
    systems: NodeSystems,
}

struct NodeClosures<D: SignalData> {
    f: SmallVec<[SubscriberClosure<D>; 1]>,
    /// Bookkeeping the scan never looks at, kept in a separate array so it
    /// stays out of the way of the dispatch loop.
    subs: SmallVec<[Subscription; 1]>,
}

/// Who a subscription belongs to, and what it came through.
///
/// One array of pairs rather than two parallel arrays, and that is not a
/// stylistic choice: `SubscriberSet` is fetched per row by the scan, so its size
/// is on the hot path. Measured with `smallvec/union`, which is what this crate
/// resolves to, per group:
///
/// |                          | x86-64 | wasm32 |
/// |--------------------------|--------|--------|
/// | `owners` alone           |     24 |     16 |
/// | two parallel arrays      |     48 |     32 |
/// | merged pairs (this)      |     24 |     24 |
///
/// So on 64-bit the signal rides along for free — the inline array is smaller
/// than the heap `(ptr, len)` it shares a union with, so widening it changes
/// nothing. On wasm32 that pointer pair is only 8 bytes and no longer dominates,
/// so this does cost 8 bytes per group (16 per `SubscriberSet`). Still half what
/// two arrays would cost, but not free — worth knowing before adding a fourth
/// piece of per-subscription bookkeeping.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Subscription {
    /// The entity this subscription's lifetime belongs to.
    owner: Entity,
    /// The signal it came through, or [`SHARED`].
    signal: Entity,
}

/// A subscription belonging to no single signal, and so never moved by a
/// repoint.
///
/// Derived marks are the only ones: `subscribe_derived` deliberately folds two
/// signals pointing at one target into a single mark, and tears down through
/// [`unsubscribe_source`](derived::unsubscribe_source) — which already knows how
/// to leave a shared mark alone — rather than through a repoint.
const SHARED: Entity = Entity::PLACEHOLDER;

/// Parallel to [`NodeClosures`], but holding the *entity* a mapper system is
/// parked on rather than the system itself.
///
/// A system can't live in the subscriber set directly: running one needs `&mut`
/// and the scan only ever sees the set through a shared reference. Parking it on
/// its own entity also means two subscribers can point at one instance instead
/// of each carrying a private `QueryState`.
struct NodeSystems {
    f: SmallVec<[Entity; 1]>,
    subs: SmallVec<[Subscription; 1]>,
}

/// A mapper system parked on its own entity.
#[derive(Component)]
pub(crate) struct RegisteredSignalSystem<D: SignalData> {
    /// Key into [`SharedSignalSystems`], so a shared registration can drop out
    /// of the map when its last subscriber goes.
    pub(crate) id: TypeId,
    /// Subscribers currently pointing here. Zero means despawn.
    pub(crate) users: usize,
    /// The `Mutex` is what turns the scan's `&RegisteredSignalSystem` into the
    /// `&mut` `run_readonly` wants. It is never contended: the scan runs mappers
    /// one at a time, and a read-only mapper cannot re-enter the scan.
    pub(crate) system: Mutex<BoxedReadOnlySystem<TargetedSignalCtx<'static, D>>>,
}

/// Mapper systems shared by every subscriber that uses them.
///
/// A mapper is shareable when the value it was built from is zero-sized — a
/// plain `fn` item or a non-capturing closure — because then two subscribers
/// asking for "the same" mapper really are asking for the same thing. A
/// capturing closure gets a registration of its own; the capture is the
/// difference.
#[derive(Resource, Default)]
pub(crate) struct SharedSignalSystems(HashMap<TypeId, Entity>);

/// Drop one subscriber's claim on a registered mapper, despawning it if that
/// was the last one.
pub(crate) fn release_signal_system<D: SignalData>(world: &mut World, system: Entity) {
    let Some(mut registered) = world.get_mut::<RegisteredSignalSystem<D>>(system) else {
        return;
    };

    registered.users = registered.users.saturating_sub(1);
    if registered.users > 0 {
        return;
    }
    let id = registered.id;

    // Guarded rather than a blind `remove`: a unique registration shares its
    // `TypeId` with the shared one for the same mapper, and must not evict it.
    let mut shared = world.resource_mut::<SharedSignalSystems>();
    if shared.0.get(&id) == Some(&system) {
        shared.0.remove(&id);
    }

    world.despawn(system);
}

/// Input to a mapped signal.
///
/// Provides the mapped data and a command queue. Any `SystemParam`
/// that would apply deferred are not permitted.
///
/// `D` is bounded by [`QueryData`] rather than [`SignalData`] so that
/// `SignalCtx<&Foo>` can be written in a mapper's argument list, the same way
/// `Query<&Foo>` can. The elided lifetime there is late-bound, and a `'static`
/// bound on the struct would make the type ill-formed before inference ever got
/// the chance to settle it on `'static`. [`SignalCtx::In`] carries the real
/// bound.
///
/// [`SignalCtx::In`]: SystemInput
pub struct SignalCtx<'a, D: QueryData> {
    pub data: D::Item<'a, 'static>,
    pub commands: Commands<'a, 'a>,
}

impl<'a, D: SignalData> SystemInput for SignalCtx<'a, D> {
    type Param<'i> = SignalCtx<'i, D>;
    type Inner<'i> = (D::Item<'i, 'static>, Commands<'i, 'i>);

    fn wrap(this: Self::Inner<'_>) -> Self::Param<'_> {
        SignalCtx {
            data: this.0,
            commands: this.1,
        }
    }
}

/// [`SignalCtx`] plus the entity the mapper's output is written to.
///
/// What the scan actually runs, and never what a user writes. The scan is
/// generic over `D` alone, so it cannot insert an output type it can't name;
/// the mapper is wrapped in an adapter that does the insert itself and reports
/// `Out = ()`. The target rides in through the input rather than being captured
/// because a shared registration serves many subscribers.
///
/// Only `Inner` is ever built: the wrapped form exists because `SystemInput`
/// demands one, and nothing takes this as a function parameter.
#[expect(dead_code, reason = "the wrapped form is never handed to a function")]
pub(crate) struct TargetedSignalCtx<'a, D: QueryData> {
    pub(crate) target: Entity,
    pub(crate) data: D::Item<'a, 'static>,
    pub(crate) commands: Commands<'a, 'a>,
}

impl<'a, D: SignalData> SystemInput for TargetedSignalCtx<'a, D> {
    type Param<'i> = TargetedSignalCtx<'i, D>;
    type Inner<'i> = (Entity, D::Item<'i, 'static>, Commands<'i, 'i>);

    fn wrap(this: Self::Inner<'_>) -> Self::Param<'_> {
        TargetedSignalCtx {
            target: this.0,
            data: this.1,
            commands: this.2,
        }
    }
}

impl<D: SignalData> SubscriberSet<D>
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    fn add(mut world: DeferredWorld, _: HookContext) {
        world.resource_mut::<ReactiveSystems>().register::<D>();
    }

    fn push(&mut self, owner: Entity, signal: Entity, closure: SubscriberClosure<D>) {
        self.closures.f.push(closure);
        self.closures.subs.push(Subscription { owner, signal });
    }

    /// Point `owner` at an already-registered mapper system.
    fn push_system(&mut self, owner: Entity, signal: Entity, system: Entity) {
        self.systems.f.push(system);
        self.systems.subs.push(Subscription { owner, signal });
    }

    /// Take every subscription that came through `signal`, leaving the rest.
    ///
    /// The move half of a repoint: a target can carry subscriptions from any
    /// number of signals, and only this one's may follow it to its new target.
    /// [`SHARED`] entries are never taken.
    fn take_signal(&mut self, signal: Entity) -> TakenSubscriptions<D> {
        let mut taken = TakenSubscriptions::default();

        let mut index = 0;
        while index < self.closures.subs.len() {
            if self.closures.subs[index].signal == signal {
                let owner = self.closures.subs.remove(index).owner;
                taken.closures.push((owner, self.closures.f.remove(index)));
            } else {
                index += 1;
            }
        }

        let mut index = 0;
        while index < self.systems.subs.len() {
            if self.systems.subs[index].signal == signal {
                let owner = self.systems.subs.remove(index).owner;
                taken.systems.push((owner, self.systems.f.remove(index)));
            } else {
                index += 1;
            }
        }

        taken
    }

    /// Drop every subscription `owner` holds on this source, returning the
    /// mapper systems it was using so the caller can release its claim on them.
    ///
    /// Callers hold at most one closure per (owner, source) pair, so this is the
    /// whole of an unsubscribe — see `subscribe_derived`, which folds two signals
    /// pointing at the same entity into a single mark.
    fn remove_owner(&mut self, owner: Entity) -> SmallVec<[Entity; 1]> {
        let mut index = 0;
        while index < self.closures.subs.len() {
            if self.closures.subs[index].owner == owner {
                self.closures.subs.remove(index);
                drop(self.closures.f.remove(index));
            } else {
                index += 1;
            }
        }

        let mut released = SmallVec::new();
        let mut index = 0;
        while index < self.systems.subs.len() {
            if self.systems.subs[index].owner == owner {
                self.systems.subs.remove(index);
                released.push(self.systems.f.remove(index));
            } else {
                index += 1;
            }
        }
        released
    }

    /// Re-attach subscriptions taken from another target by [`take_signal`].
    ///
    /// [`take_signal`]: Self::take_signal
    fn restore(&mut self, signal: Entity, taken: &mut TakenSubscriptions<D>) {
        for (owner, closure) in taken.closures.drain(..) {
            self.push(owner, signal, closure);
        }
        for (owner, system) in taken.systems.drain(..) {
            self.push_system(owner, signal, system);
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.closures.f.len()
    }

    #[cfg(test)]
    fn system_len(&self) -> usize {
        self.systems.f.len()
    }
}

impl<D: SignalData> Default for SubscriberSet<D>
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    fn default() -> Self {
        Self {
            closures: NodeClosures {
                f: SmallVec::new(),
                subs: SmallVec::new(),
            },
            systems: NodeSystems {
                f: SmallVec::new(),
                subs: SmallVec::new(),
            },
        }
    }
}

/// Subscriptions lifted off one target and not yet placed on another.
///
/// Held apart from the world because the move needs `&mut World` for the target
/// on each end and cannot borrow either set across the other.
pub(crate) struct TakenSubscriptions<D: SignalData> {
    closures: SmallVec<[(Entity, SubscriberClosure<D>); 1]>,
    systems: SmallVec<[(Entity, Entity); 1]>,
}

impl<D: SignalData> TakenSubscriptions<D> {
    pub(crate) fn is_empty(&self) -> bool {
        self.closures.is_empty() && self.systems.is_empty()
    }

    pub(crate) fn closures(&self) -> impl Iterator<Item = &(Entity, SubscriberClosure<D>)> {
        self.closures.iter()
    }

    pub(crate) fn systems(&self) -> impl Iterator<Item = &(Entity, Entity)> {
        self.systems.iter()
    }
}

impl<D: SignalData> Default for TakenSubscriptions<D> {
    fn default() -> Self {
        Self {
            closures: SmallVec::new(),
            systems: SmallVec::new(),
        }
    }
}

pub trait SignalExt<'w, 's> {
    fn signal<D>(&mut self) -> SourceSignalView<'_, 'w, 's, D>
    where
        D: SignalData,
        for<'ww, 'ss> D::Item<'ww, 'ss>: Copy;

    /// Spawn a derived signal: a closure over any number of other signals,
    /// re-run whenever one of them changes and never more than once per pass.
    ///
    /// The result is an ordinary [`SourceSignal`] over `O`, so a derived signal
    /// can be mapped or read by another derived signal with no extra machinery.
    fn derive<F, O>(&mut self, eval: F) -> SourceSignal<&'static O>
    where
        F: Fn(&source::SignalStore) -> Result<O, ReactError> + Send + Sync + 'static,
        O: Component;

    /// Build a [`Cell`]: a signal whose value lives in the handle rather than
    /// in a component, so it can be read and written from anywhere without a
    /// world and without waiting for a flush.
    ///
    /// This is what local view state wants — a focus flag, a drag offset, a
    /// fetch's in-flight bool. Anything the rest of the ECS also reads should
    /// be a component and an ordinary [`signal`](SignalExt::signal) instead.
    #[must_use]
    fn cell<T>(&mut self, value: T) -> Cell<T>
    where
        T: Send + Sync + 'static;

    /// Build a keyed list over a collection signal.
    ///
    /// Nothing here needs the queue — the returned component is inert until it
    /// is placed on a container — so this is only [`ReactiveList::new`] spelled
    /// to match the rest of the builders.
    #[must_use]
    fn list<R, S, K, F, G, B>(
        &mut self,
        source: SourceSignal<&'static S>,
        key: F,
        row: G,
    ) -> ReactiveList<R>
    where
        R: Relationship,
        S: Component + ListSource,
        S::Item: Clone + PartialEq + Send + Sync + 'static,
        K: Eq + core::hash::Hash + Clone + Send + Sync + 'static,
        F: Fn(&S::Item) -> K + Send + Sync + 'static,
        G: Fn(&S::Item, &mut Commands) -> B + Send + Sync + 'static,
        B: Bundle,
    {
        ReactiveList::new(source, key, row)
    }
}

impl<'w, 's> SignalExt<'w, 's> for Commands<'w, 's> {
    fn signal<D>(&mut self) -> SourceSignalView<'_, 'w, 's, D>
    where
        D: SignalData,
        for<'ww, 'ss> D::Item<'ww, 'ss>: Copy,
    {
        let signal = SourceSignal::from_commands(self);
        SourceSignalView(signal, self)
    }

    fn derive<F, O>(&mut self, eval: F) -> SourceSignal<&'static O>
    where
        F: Fn(&source::SignalStore) -> Result<O, ReactError> + Send + Sync + 'static,
        O: Component,
    {
        derived::spawn_derive(self, eval)
    }

    fn cell<T>(&mut self, value: T) -> Cell<T>
    where
        T: Send + Sync + 'static,
    {
        Cell::new(self, value)
    }
}

#[derive(Debug)]
pub enum ReactError {
    NotWatched(Entity),
    SourceEntityError(QueryEntityError),
    TargetEntityError(QueryEntityError),
    TargetQueryError(QueryAccessError),
}

impl core::fmt::Display for ReactError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotWatched(e) => write!(f, "entity {e:?} is not reactively watched"),
            Self::SourceEntityError(e) => write!(f, "failed to fetch source entity: {e}"),
            Self::TargetEntityError(e) => write!(f, "failed to fetch signal target: {e}"),
            Self::TargetQueryError(e) => write!(f, "signal target doesn't match: {e}"),
        }
    }
}

impl core::error::Error for ReactError {}

#[derive(Resource, Default)]
pub struct ReactiveSystems {
    /// The set of all systems ever registered.
    registered: HashSet<TypeId>,
    /// Systems that haven't been drained.
    systems: Vec<(TypeId, SignalSystem)>,
}

impl ReactiveSystems {
    /// Idempotently register a basic signal system.
    pub fn register<D>(&mut self) -> bool
    where
        D: SignalData,
        for<'w, 's> D::Item<'w, 's>: Copy,
    {
        let id = TypeId::of::<D>();
        if self.registered.insert(id) {
            self.systems
                .push((id, <D as IntoSignalSystem>::into_signal_system()));
            true
        } else {
            false
        }
    }
}

/// Systems that have been drained from the `ReactiveSystems` resource.
#[derive(Resource)]
struct InnerReactiveSystems {
    /// Held in registration order rather than a `TypeIdMap`, because that order
    /// is a usable approximation of dependency order: a signal's own type is
    /// registered before anything that maps out of it, so most chains propagate
    /// fully within a single pass instead of costing an extra one.
    source_systems: Vec<(TypeId, SignalSystem)>,
    /// The single, non-generic pass that settles derived nodes.
    derived: SignalSystem,
}

impl FromWorld for InnerReactiveSystems {
    fn from_world(world: &mut World) -> Self {
        let mut derived: SignalSystem = Box::new(IntoSystem::into_system(run_derived_nodes));
        derived.initialize(world);

        Self {
            source_systems: Vec::new(),
            derived,
        }
    }
}

/// Maximum settle passes per frame.
///
/// A frame that changes nothing costs one pass. Any frame that dispatches costs
/// at least two: the second is what proves the graph settled. Reaching the cap
/// means a subscriber keeps writing something another subscriber watches — a
/// cycle.
pub const REACTION_LIMIT: usize = 16;

/// Run the reactive systems until no subscriber fires.
///
/// Effects write through `Commands`, so a pass can dirty state that the pass's
/// own scans already walked past — a chain of mapped signals is the ordinary
/// case. Each pass's `Changed` filters are relative to that system's own last
/// run, so pass N+1 only sees what pass N wrote.
pub fn run_reactive_schedule(world: &mut World) {
    settle_reactive(world);
}

/// [`run_reactive_schedule`], returning how many passes it took to settle.
///
/// One means nothing reacted. Two is the floor for any frame that dispatched --
/// the second pass is what proves there is nothing left. More than that means a
/// signal chain propagated across passes.
pub fn settle_reactive(world: &mut World) -> usize {
    let mut work = ReactiveWork::default();
    let mut errors = Vec::new();
    let mut passes = 0;

    for pass in 0..REACTION_LIMIT {
        // New signal types can be registered by the previous pass's commands
        // (a subscriber that spawns more reactive entities), so drain every
        // pass rather than once up front.
        drain_new_systems(world);

        work.dispatched = 0;
        let pending = core::mem::take(&mut world.resource_mut::<PendingNodes>().0);
        work.dirty.extend(pending);

        // Before the scans rather than alongside them: a cell write has already
        // happened by the time anyone can see it, so the nodes it wakes belong
        // in this pass's dirty list, not the next one's.
        drain_dirty_cells(world, &mut work);

        world.resource_scope(|world, mut inner: Mut<InnerReactiveSystems>| {
            let inner = &mut *inner;
            for (_, system) in inner.source_systems.iter_mut() {
                if let Err(e) = system.run(&mut work, world) {
                    errors.push(e);
                }
            }

            if let Err(e) = inner.derived.run(&mut work, world) {
                errors.push(e);
            }
        });

        // Outside the scope, and last: effects are arbitrary systems, so they
        // run only once every read-only pass has finished and flushed what it
        // wrote.
        run_pending_effects(world, &mut work);

        passes += 1;

        if work.dispatched == 0 {
            break;
        }

        if pass == REACTION_LIMIT - 1 {
            log::warn!(
                "signal2: reactive schedule did not settle in {REACTION_LIMIT} passes \
                 ({} dispatches still pending); a subscriber is likely writing to \
                 something it watches",
                work.dispatched
            );
        }
    }

    if !errors.is_empty() {
        // TOOD: just forward these to the error handler
        log::error!("Failed to evaluate all reactive systems: {errors:#?}");
    }

    passes
}

/// Move newly registered systems into the run list, initializing each.
fn drain_new_systems(world: &mut World) {
    if world.resource::<ReactiveSystems>().systems.is_empty() {
        return;
    }

    world.resource_scope(|world, mut outer: Mut<ReactiveSystems>| {
        let now = world.change_tick();
        for (_, system) in outer.systems.iter_mut() {
            system.initialize(world);
            // `initialize` backdates `last_run` so that everything in the world
            // reads as changed, which is the right default for a system that has
            // never run. It is wrong here: every subscriber is brought up to date
            // the moment it attaches, so a system starting from "everything
            // changed" would redo that work for every live entity of its type —
            // the whole existing population dispatched a second time on the frame
            // the type is first used.
            system.set_last_run(now);
        }

        let drained = outer.systems.drain(..).collect::<Vec<_>>();
        world
            .resource_mut::<InnerReactiveSystems>()
            .source_systems
            .extend(drained);
    });
}
