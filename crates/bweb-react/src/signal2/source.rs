use std::{
    marker::PhantomData,
    sync::{Arc, Mutex, PoisonError},
};

use bevy_ecs::{lifecycle::HookContext, prelude::*, system::SystemParam, world::DeferredWorld};

use crate::signal2::{
    ReactError, ReactiveSystems, SHARED, SignalData, SubscriberSet, TakenSubscriptions,
    derived::{
        NodeLevel, NodeSource, NodeSources, NodeSubscribers, PendingNodes, raise_level,
        unsubscribe_source,
    },
    dynamic::Signal,
    effect::{Effect, reads_post_flush},
    mapped::{dispatch_once, run_registered_mapper},
};

pub struct SourceSignal<D> {
    pub(crate) data: Arc<InnerData>,
    marker: PhantomData<fn() -> D>,
}

impl<D> Clone for SourceSignal<D> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            marker: PhantomData,
        }
    }
}

pub(crate) struct InnerData {
    pub(crate) entity: Entity,
}

/// The entity a signal reads its value from.
///
/// A derived node watches itself: its value is a component on its own entity.
///
/// Present on a signal entity from the moment it is spawned, holding
/// [`UNBOUND`](Self::UNBOUND) until something points it somewhere. That costs
/// nothing and buys two things:
///
/// - Binding is an in-place write rather than an insert, so a signal entity is
///   built in one archetype instead of being moved out of an empty one. Spawning
///   is the hot path for a UI that builds subtrees, and this is measured by
///   `signal2/spawn`.
/// - A binding can be established from inside a component hook, which cannot
///   make structural changes but can write a component that is already there.
///   [`WatchTarget`] depends on that: it has to land before the *queued*
///   subscription commands of anything sharing its bundle, and going through the
///   queue itself would make correctness depend on component order within the
///   bundle.
#[derive(Component, Clone, Copy)]
pub struct Watching(pub Entity);

impl Watching {
    /// A signal that has not been pointed at anything yet.
    pub const UNBOUND: Self = Self(Entity::PLACEHOLDER);

    /// The watched entity, or `None` while unbound.
    pub fn target(&self) -> Option<Entity> {
        (self.0 != Entity::PLACEHOLDER).then_some(self.0)
    }
}

/// The entity `signal` currently reads from, or `None` if it is unbound.
pub(crate) fn watched(world: &World, signal: Entity) -> Option<Entity> {
    world.get::<Watching>(signal).and_then(Watching::target)
}

/// Subscription work parked on a signal entity until it has a target.
///
/// Every subscription path resolves `Watching` once and pushes into the target's
/// [`SubscriberSet`]. With [`watch`](SourceSignalView::watch) the binding is
/// queued when the signal is built, so it is always in place first. With
/// [`watch_bundle`](SourceSignalView::watch_bundle) it is not: the host may be
/// spawned after something that already maps off it, and hooks queued during a
/// spawn are flushed before the outer queue continues — so the subscriber can
/// genuinely arrive first.
///
/// Dropping it there (which is what used to happen) makes the failure depend on
/// spawn order and reports it as nothing but a warning. Parking it instead makes
/// order irrelevant: whoever arrives first waits, and [`bind`] settles the debt.
#[derive(Component, Default)]
pub(crate) struct PendingBindings(Vec<BindingThunk>);

type BindingThunk = Box<dyn FnOnce(&mut World, Entity) + Send + Sync>;

/// Park `subscribe` until `signal` is pointed at something, to be run with the
/// entity it ends up pointing at.
pub(crate) fn park<F>(world: &mut World, signal: Entity, subscribe: F)
where
    F: FnOnce(&mut World, Entity) + Send + Sync + 'static,
{
    let Ok(mut signal) = world.get_entity_mut(signal) else {
        return;
    };
    signal
        .entry::<PendingBindings>()
        .or_default()
        .into_mut()
        .0
        .push(Box::new(subscribe));
}

/// Point `signal` at `target` and run whatever was waiting on that.
///
/// The insert is a fallback for a signal entity built outside the usual queue
/// ordering — [`SourceSignal::from_commands`] spawns the component, but a caller
/// that reaches the binding before that spawn has applied would otherwise lose
/// it.
pub(crate) fn bind(world: &mut World, signal: Entity, target: Entity) {
    if let Some(mut watching) = world.get_mut::<Watching>(signal) {
        watching.0 = target;
    } else if let Ok(mut signal) = world.get_entity_mut(signal) {
        signal.insert(Watching(target));
    } else {
        return;
    }

    // Taken rather than drained in place: a thunk can park another one, and the
    // borrow cannot be held across the `&mut World` each of them wants.
    let Some(pending) = world
        .get_mut::<PendingBindings>(signal)
        .map(|mut pending| core::mem::take(&mut pending.0))
    else {
        return;
    };

    for subscribe in pending {
        subscribe(world, target);
    }
}

pub struct SourceSignalView<'a, 'w, 's, D>(
    pub(crate) SourceSignal<D>,
    pub(crate) &'a mut Commands<'w, 's>,
);

impl<'a, 'w, 's, D: SignalData> SourceSignalView<'a, 'w, 's, D>
where
    for<'x, 'y> D::Item<'x, 'y>: Copy,
{
    /// Point the signal at a fixed entity.
    pub fn watch(self, entity: Entity) -> SourceSignal<D> {
        let signal_entity = self.0.data.entity;
        self.1.queue(move |world: &mut World| {
            bind(world, signal_entity, entity);
            world
                .entity_mut(entity)
                .entry::<SubscriberSet<D>>()
                .or_default();
        });
        self.0
    }

    /// Point the signal at whatever entity `binding` currently names, following
    /// it when that changes.
    ///
    /// For a view whose subject is itself reactive — the selected item, the
    /// hovered row, the entity a drag is over:
    ///
    /// ```ignore
    /// let selected: Cell<Entity> = commands.cell(first);
    /// let name = commands.signal::<&Name>().watch_signal(selected.clone());
    /// commands.spawn(name.map(render));   // follows `selected` from now on
    /// ```
    ///
    /// A constant binding takes [`watch`](Self::watch)'s path instead, so
    /// passing a plain `Entity` here costs nothing extra — no effect, no node,
    /// no system registration. Only a genuinely reactive binding pays for the
    /// machinery that can move it.
    ///
    /// # Cost of a rebind
    ///
    /// Moving the binding moves the mapped subscribers with it and dispatches
    /// each once against the new target. Derived readers are re-evaluated rather
    /// than moved, which they do on the pass after the rebind — so a frame in
    /// which the binding changes costs one extra settle pass. See [`repoint`].
    pub fn watch_signal(self, binding: impl Into<Signal<Entity>>) -> SourceSignal<D> {
        let binding = binding.into();

        // A binding that cannot change needs none of what follows, and this is
        // the case a generic caller hits whenever it passes a literal through.
        if let Some(&entity) = binding.as_value() {
            return self.watch(entity);
        }

        let signal = self.0.data.entity;
        // Parked on the signal entity itself, which is otherwise inert — a
        // source signal has no `DerivedNode` for the effect to overwrite, unlike
        // a derived one. Reading `binding` here is what subscribes the effect to
        // it; the rest is machinery the effect tier already provides.
        self.1.entity(signal).insert(Effect::new(
            move |In(node): In<Entity>, store: SignalStore, mut commands: Commands| {
                let Ok(target) = binding.get(&store) else {
                    return;
                };
                let target = *target;
                commands.queue(move |world: &mut World| {
                    repoint::<D>(world, node, Some(target));
                });
            },
        ));

        self.0
    }

    /// Point the signal at whichever entity this bundle is inserted onto.
    ///
    /// For a view that watches its own entity — the component it reads is on the
    /// same thing the signal's output is arranged around, and neither entity is
    /// known until the spawn happens:
    ///
    /// ```ignore
    /// let hovered = commands.signal::<&Hovered>().watch_bundle();
    /// commands.spawn((Button, hovered.map(highlight), hovered));
    /// ```
    ///
    /// The returned value derefs to the [`SourceSignal`], so it can be mapped
    /// and read like any other before being inserted.
    #[must_use]
    pub fn watch_bundle(self) -> WatchTarget<D> {
        WatchTarget(self.0)
    }
}

/// Binds a signal to the entity it is inserted onto.
///
/// Built by [`SourceSignalView::watch_bundle`]. Derefs to the [`SourceSignal`]
/// it binds, so one value serves as both the handle and the bundle.
pub struct WatchTarget<D>(SourceSignal<D>);

impl<D> WatchTarget<D> {
    /// The signal this binds, for a caller that would rather be explicit than
    /// lean on the `Deref`.
    pub fn signal(&self) -> SourceSignal<D> {
        self.0.clone()
    }
}

impl<D> core::ops::Deref for WatchTarget<D> {
    type Target = SourceSignal<D>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<D> Clone for WatchTarget<D> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Written by hand rather than derived because the `D::Item: Copy` bound the
/// hooks need cannot be stated on the struct without infecting every use of the
/// handle.
impl<D: SignalData> bevy_ecs::component::Component for WatchTarget<D>
where
    Self: Send + Sync + 'static,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    const STORAGE_TYPE: bevy_ecs::component::StorageType = bevy_ecs::component::StorageType::Table;

    type Mutability = bevy_ecs::component::Mutable;

    fn register_required_components(
        _requiree: bevy_ecs::component::ComponentId,
        _required_components: &mut bevy_ecs::component::RequiredComponentsRegistrator,
    ) {
    }

    fn clone_behavior() -> bevy_ecs::component::ComponentCloneBehavior {
        bevy_ecs::component::ComponentCloneBehavior::Ignore
    }

    fn relationship_accessor() -> Option<bevy_ecs::relationship::ComponentRelationshipAccessor<Self>>
    {
        None
    }

    fn on_insert() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        Some(|mut world: DeferredWorld, ctx: HookContext| {
            let entity = ctx.entity;
            let Some(target) = world.get::<WatchTarget<D>>(entity) else {
                return;
            };
            let signal = target.0.data.entity;

            // Written inline, and this is the whole reason `Watching` is spawned
            // unbound rather than inserted on binding. Every subscription path
            // resolves `Watching` from inside a *queued* command. Going through
            // the queue here too would make a bundle holding both this and a
            // mapped signal depend on which of the two hooks ran first.
            //
            // This covers subscribers whose command has not run yet. Ones that
            // already ran are parked in `PendingBindings`, and draining those
            // needs `&mut World` — hence the queued `bind` below, which repeats
            // this write harmlessly and settles them.
            if let Some(mut watching) = world.get_mut::<Watching>(signal) {
                watching.0 = entity;
            }

            world.commands().queue(move |world: &mut World| {
                bind(world, signal, entity);
                // Structural, so it could not have been done inline. Nothing
                // reads it before the scan does, which is well after this.
                if let Ok(mut entity) = world.get_entity_mut(entity) {
                    entity.entry::<SubscriberSet<D>>().or_default();
                }
            });
        })
    }

    /// Unbind when the component goes away, so a signal outliving its host does
    /// not keep reporting the host's last value.
    ///
    /// `on_replace` is the one hook that fires for all three endings —
    /// overwritten, removed, despawned.
    ///
    /// Subscribers go with it, parked until the next binding, so moving a
    /// `WatchTarget` from one entity to another keeps them intact.
    fn on_replace() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        Some(|mut world: DeferredWorld, ctx: HookContext| {
            let Some(target) = world.get::<WatchTarget<D>>(ctx.entity) else {
                return;
            };
            let signal = target.0.data.entity;

            // Guarded: the signal may already have been pointed somewhere else,
            // in which case this component is stale and unbinding would undo a
            // binding it does not own.
            if world.get::<Watching>(signal).and_then(Watching::target) != Some(ctx.entity) {
                return;
            }

            world
                .commands()
                .queue(move |world: &mut World| repoint::<D>(world, signal, None));
        })
    }
}

impl<D> SourceSignal<D>
where
    D: SignalData,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    pub fn from_commands(commands: &mut Commands) -> Self {
        // Spawned already carrying `Watching`, so binding never moves the
        // entity between archetypes — see [`Watching`].
        let source_entity = commands.spawn(Watching::UNBOUND).id();
        let data = Arc::new(InnerData {
            entity: source_entity,
        });
        commands.queue(|world: &mut World| {
            if world.resource_mut::<ReactiveSystems>().register::<D>() {
                D::register_removal_wakeups::<D>(&mut world.commands());
            }
        });

        SourceSignal {
            data,
            marker: PhantomData,
        }
    }

    /// Wrap an entity that already has [`Watching`].
    pub(crate) fn from_entity(entity: Entity) -> Self {
        SourceSignal {
            data: Arc::new(InnerData { entity }),
            marker: PhantomData,
        }
    }

    /// The signal's entity, to which downstream nodes subscribe.
    pub fn entity(&self) -> Entity {
        self.data.entity
    }
}

/// Point `signal` at `to`, carrying its subscribers across.
///
/// Every subscription resolves `Watching` once, at attach time, and nothing in
/// the graph re-reads it — so moving the binding is not enough on its own. The
/// two kinds of subscriber are moved differently:
///
/// - A mapped signal's closure or system captures its owner and its mapper, not
///   its source, so it can be lifted off one [`SubscriberSet`] and dropped onto
///   another unchanged. It is then dispatched once, for the same reason a fresh
///   subscriber is: its new target has almost certainly not changed this frame.
/// - A derived node is *not* moved. Its edge is dropped and the node is queued
///   to re-evaluate, at which point its read reports the signal again and
///   [`subscribe_derived`] wires it to the new target. That reuses the lazy path
///   the derived tier already has instead of hand-maintaining `NodeSources`,
///   `NodeSubscribers` and the shared-mark rule at a second site.
///
/// Unbinding (`to` of `None`) parks the lifted subscriptions rather than
/// dropping them, so a signal that is unbound and later rebound — a
/// [`WatchTarget`] moved from one entity to another — comes back intact.
pub(crate) fn repoint<D>(world: &mut World, signal: Entity, to: Option<Entity>)
where
    D: SignalData,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    let from = watched(world, signal);
    if from == to {
        return;
    }

    if let Some(from) = from {
        // Candidates are the derived nodes subscribed to the *old target*, which
        // is a short list and already maintained. Only those holding an edge
        // through this particular signal are ours to drop.
        let candidates = world
            .get::<NodeSubscribers>(from)
            .map(|subscribers| subscribers.0.clone())
            .unwrap_or_default();

        for node in candidates {
            if !world
                .get::<NodeSources>(node)
                .is_some_and(|sources| sources.contains(signal))
            {
                continue;
            }
            unsubscribe_source(world, node, signal);
            // The node's value came from the old target, so wiring the new edge
            // is not enough — it has to run again.
            world.resource_mut::<PendingNodes>().0.push(node);
        }

        let taken = world
            .get_mut::<SubscriberSet<D>>(from)
            .map(|mut subs| subs.take_signal(signal));

        if let Some(mut taken) = taken
            && !taken.is_empty()
        {
            // Parked rather than placed directly, so that binding and rebinding
            // go through one path — `bind` drains this immediately when `to` is
            // `Some`, and holds it until the next binding when it is not.
            park(world, signal, move |world: &mut World, target: Entity| {
                restore_subscriptions::<D>(world, signal, target, &mut taken);
            });
        }
    }

    match to {
        Some(to) => {
            bind(world, signal, to);
            if let Ok(mut to) = world.get_entity_mut(to) {
                to.entry::<SubscriberSet<D>>().or_default();
            }
        }
        None => {
            if let Some(mut watching) = world.get_mut::<Watching>(signal) {
                *watching = Watching::UNBOUND;
            }
        }
    }
}

/// Place lifted subscriptions on `target` and bring each one up to date.
fn restore_subscriptions<D>(
    world: &mut World,
    signal: Entity,
    target: Entity,
    taken: &mut TakenSubscriptions<D>,
) where
    D: SignalData,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    // Dispatched before being placed, not after: a closure runs against the new
    // target either way, and doing it here keeps the borrow of the set out of
    // the way of the `&mut World` each dispatch wants.
    for (_, closure) in taken.closures() {
        dispatch_once::<D>(world, target, closure);
    }
    for (owner, system) in taken.systems() {
        run_registered_mapper::<D>(world, *system, *owner, target);
    }

    let Ok(mut target_entity) = world.get_entity_mut(target) else {
        return;
    };
    let mut subs = target_entity
        .entry::<SubscriberSet<D>>()
        .or_default()
        .into_mut();
    subs.restore(signal, taken);
}

/// Subscribes `node` to the source behind `signal`, wiring only what is missing.
///
/// Recorded as a plain function pointer by [`SignalStore::record`] so the
/// derived pass can re-subscribe without knowing `D`.
pub(crate) type SubscribeFn = fn(&mut World, signal: Entity, node: Entity);

/// The inverse, recorded on the edge itself so it can be undone the same way.
///
/// Takes the resolved target rather than the signal: by the time an edge is torn
/// down the signal may have been repointed or despawned, and the mark that has to
/// go is the one on the entity the edge was wired against.
pub(crate) type UnsubscribeFn = fn(&mut World, target: Entity, node: Entity);

pub(crate) fn subscribe_derived<D>(world: &mut World, signal: Entity, node: Entity)
where
    D: SignalData,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    let Some(target) = watched(world, signal) else {
        // The source hasn't been pointed at anything yet. Waiting for the
        // binding is the only option that terminates: the read that reported
        // this edge failed, so the node has no edge to be woken through and
        // would never evaluate again on its own.
        park(world, signal, move |world: &mut World, _target| {
            subscribe_derived::<D>(world, signal, node);
            // The failed read means the node is holding an error rather than a
            // value, so wiring the edge is not enough — it has to run again.
            world.resource_mut::<PendingNodes>().0.push(node);
        });
        return;
    };

    // One past the deepest input reachable so far. Plain components are level
    // zero, so a node reading only world state sorts first in a pass.
    //
    // Recomputed on every reported read, not just new ones: a node can be
    // evaluated before the input it reads has a level of its own, and would
    // otherwise keep that too-shallow level forever. The raise walks downstream,
    // so subscribers wired before this edge existed deepen with it.
    let source_level = world.get::<NodeLevel>(target).map_or(0, |level| level.0);
    raise_level(world, node, source_level.saturating_add(1));

    if world
        .get::<NodeSources>(node)
        .is_some_and(|sources| sources.contains(signal))
    {
        return;
    }

    // A brand new edge means the node just read something it was not subscribed
    // to, and that read can be stale: the value it wanted may still be sitting in
    // the command queue as an upstream node's output. This is the same debt a
    // mapped signal settles inline when it attaches, except a derived node cannot
    // pay it until after it has run. It re-runs once instead.
    //
    // An effect has nothing to pay: it already reads post-flush.
    if !reads_post_flush(world, node) {
        world.resource_mut::<PendingNodes>().0.push(node);
    }

    // Two signals can point at the same entity. They are distinct edges, but they
    // share one mark: a second closure would only mark the node twice for one
    // change, and would make the two edges impossible to tell apart at teardown.
    let marked = world
        .get::<NodeSources>(node)
        .is_some_and(|sources| sources.watches(target));

    if !marked {
        {
            let mut target_entity = world.entity_mut(target);
            let mut subs = target_entity
                .entry::<SubscriberSet<D>>()
                .or_default()
                .into_mut();
            // The scan doesn't evaluate a derived node, it only marks it: that is
            // what collapses N changed inputs into one evaluation.
            //
            // Attributed to no signal: the mark above is deliberately shared by
            // every signal of this node's that resolves here, so a repoint must
            // not carry it off. `unsubscribe_source` is what takes it down, and
            // it already knows to leave a mark another edge still needs.
            subs.push(
                node,
                SHARED,
                Box::new(move |_data, ctx| ctx.work.mark_dirty(node)),
            );
        }

        let mut target_entity = world.entity_mut(target);
        let mut subscribers = target_entity
            .entry::<NodeSubscribers>()
            .or_default()
            .into_mut();
        subscribers.0.push(node);
    }

    if let Some(mut sources) = world.get_mut::<NodeSources>(node) {
        sources.0.push(NodeSource {
            signal,
            target,
            unsubscribe: unsubscribe_derived::<D>,
        });
    }
}

/// Drop `node`'s mark from `target`, along with the reverse edge levels use.
pub(crate) fn unsubscribe_derived<D>(world: &mut World, target: Entity, node: Entity)
where
    D: SignalData,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    let Ok(mut target_entity) = world.get_entity_mut(target) else {
        // The target was despawned, which took its subscribers with it.
        return;
    };

    if let Some(mut subs) = target_entity.get_mut::<SubscriberSet<D>>() {
        subs.remove_owner(node);
    }

    if let Some(mut subscribers) = target_entity.get_mut::<NodeSubscribers>() {
        subscribers.0.retain(|subscriber| *subscriber != node);
    }
}

/// One signal a node touched, paired with the way to subscribe to it.
pub(crate) type Read = (Entity, SubscribeFn);

/// Signals read by the evaluation currently in flight.
///
/// This started life as a `Local` on [`SignalStore`], which is the right home
/// while every reader is a derived closure sharing the pass's one store. An
/// effect is a whole system with a store of its own, and whoever is driving the
/// evaluation has to harvest the reads afterwards — which it cannot do from
/// inside another system's params. So the buffer lives out here instead.
///
/// One buffer is enough because evaluation is strictly sequential: a node is
/// cleared, run, and harvested before the next one starts.
#[derive(Resource, Default)]
pub(crate) struct SignalReads(Mutex<Vec<Read>>);

impl SignalReads {
    /// A poisoned buffer means a node panicked mid-evaluation, which says
    /// nothing about the reads recorded before it — and a signal graph that
    /// stops tracking is worse than one carrying a stale entry, so the lock is
    /// recovered rather than propagated.
    fn buffer(&self) -> std::sync::MutexGuard<'_, Vec<Read>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn clear(&self) {
        self.buffer().clear();
    }

    pub(crate) fn take(&self) -> Vec<Read> {
        core::mem::take(&mut *self.buffer())
    }
}

/// Read access to signal values, plus the read-tracking that gives derived
/// signals their subscriptions.
///
/// Every [`SourceSignal::get`] through a store records the signal it touched,
/// so a derived node's dependencies come from what it actually read on its last
/// run rather than from a list declared up front.
#[derive(SystemParam)]
pub struct SignalStore<'w, 's> {
    values: Query<'w, 's, EntityRef<'static>>,
    reads: Res<'w, SignalReads>,
}

impl SignalStore<'_, '_> {
    pub(crate) fn clear_reads(&self) {
        self.reads.clear();
    }

    pub(crate) fn take_reads(&self) -> Vec<Read> {
        self.reads.take()
    }

    fn record<D>(&self, signal: Entity)
    where
        D: SignalData,
        for<'w, 's> D::Item<'w, 's>: Copy,
    {
        self.record_at(signal, subscribe_derived::<D>);
    }

    /// [`record`](Self::record) for a source that supplies its own way of
    /// subscribing, rather than having one picked from `D`.
    ///
    /// A [`Cell`](crate::signal2::cell::Cell) has no `D` to pick from: its value
    /// is in its handle, so there is no component to filter on and no
    /// [`SubscriberSet`] to join.
    pub(crate) fn record_at(&self, signal: Entity, subscribe: SubscribeFn) {
        self.reads.buffer().push((signal, subscribe));
    }
}

impl<D> SourceSignal<D>
where
    D: SignalData,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    pub fn get<'a>(
        &self,
        store: &'a SignalStore<'_, '_>,
    ) -> Result<D::Item<'a, 'static>, ReactError> {
        store.record::<D>(self.data.entity);
        let target = store
            .values
            .get(self.data.entity)
            .map_err(ReactError::SourceEntityError)?
            .get::<Watching>()
            .and_then(Watching::target)
            .ok_or(ReactError::NotWatched(self.data.entity))?;

        store
            .values
            .get(target)
            .map_err(ReactError::TargetEntityError)?
            .get_components::<D>()
            .map_err(ReactError::TargetQueryError)
    }
}
