use std::any::TypeId;

use bevy_app::{Plugin, PostUpdate};
use bevy_ecs::{
    entity::EntityHashSet,
    lifecycle::HookContext,
    prelude::*,
    query::{
        QueryAccessError, QueryData, QueryEntityError, QueryFilter, ReadOnlyQueryData,
        ReleaseStateQueryData,
    },
    system::{BoxedReadOnlySystem, InMut},
    world::DeferredWorld,
};
use bevy_platform::collections::HashSet;

pub mod derived;
pub mod mapped;
pub mod source;

#[cfg(test)]
mod tests;

use smallvec::SmallVec;
use source::SourceSignal;

use crate::signal2::{
    derived::{PendingNodes, run_derived_nodes},
    source::SourceSignalView,
};

pub struct ReactivePlugin;

impl Plugin for ReactivePlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.init_resource::<ReactiveSystems>()
            .init_resource::<PendingNodes>()
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
                      mut commands: Commands| {
            for (data, subs) in &query {
                for closure in &subs.closures {
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
    closures: SmallVec<[SubscriberClosure<D>; 1]>,
    /// Parallel to `closures`: the entity each subscription's lifetime belongs
    /// to, which is what makes an otherwise anonymous closure removable.
    ///
    /// Kept in its own array rather than paired with the closure so the scan's
    /// dispatch loop still walks a dense run of pointers — an owner is only ever
    /// read when a subscription is torn down, which is off the hot path.
    owners: SmallVec<[Entity; 1]>,
}

impl<D: SignalData> SubscriberSet<D>
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    fn add(mut world: DeferredWorld, _: HookContext) {
        world.resource_mut::<ReactiveSystems>().register::<D>();
    }

    fn push(&mut self, owner: Entity, closure: SubscriberClosure<D>) {
        self.closures.push(closure);
        self.owners.push(owner);
    }

    /// Drop every subscription `owner` holds on this source.
    ///
    /// Callers hold at most one closure per (owner, source) pair, so this is the
    /// whole of an unsubscribe — see `subscribe_derived`, which folds two signals
    /// pointing at the same entity into a single mark.
    fn remove_owner(&mut self, owner: Entity) {
        let mut index = 0;
        while index < self.owners.len() {
            if self.owners[index] == owner {
                self.owners.remove(index);
                drop(self.closures.remove(index));
            } else {
                index += 1;
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.closures.len()
    }
}

impl<D: SignalData> Default for SubscriberSet<D>
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    fn default() -> Self {
        Self {
            closures: SmallVec::new(),
            owners: SmallVec::new(),
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
