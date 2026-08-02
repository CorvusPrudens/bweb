use std::{
    marker::PhantomData,
    sync::{Arc, Mutex, PoisonError},
};

use bevy_ecs::{prelude::*, system::SystemParam};

use crate::signal2::{
    ReactError, ReactiveSystems, SignalData, SubscriberSet,
    derived::{NodeLevel, NodeSource, NodeSources, NodeSubscribers, PendingNodes, raise_level},
    effect::reads_post_flush,
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
#[derive(Component, Clone, Copy)]
pub struct Watching(pub Entity);

pub struct SourceSignalView<'a, 'w, 's, D>(
    pub(crate) SourceSignal<D>,
    pub(crate) &'a mut Commands<'w, 's>,
);

impl<'a, 'w, 's, D: SignalData> SourceSignalView<'a, 'w, 's, D>
where
    for<'x, 'y> D::Item<'x, 'y>: Copy,
{
    pub fn watch(self, entity: Entity) -> SourceSignal<D> {
        let signal_entity = self.0.data.entity;
        self.1.queue(move |world: &mut World| {
            world.entity_mut(signal_entity).insert(Watching(entity));
            world
                .entity_mut(entity)
                .entry::<SubscriberSet<D>>()
                .or_default();
        });
        self.0
    }
}

impl<D> SourceSignal<D>
where
    D: SignalData,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    pub fn from_commands(commands: &mut Commands) -> Self {
        let source_entity = commands.spawn_empty().id();
        let data = Arc::new(InnerData {
            entity: source_entity,
        });
        commands.queue(|world: &mut World| {
            world.resource_mut::<ReactiveSystems>().register::<D>();
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
    let Some(&Watching(target)) = world.get::<Watching>(signal) else {
        // The source hasn't been pointed at anything yet; the read that
        // recorded this will be retried on the node's next evaluation.
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
            subs.push(node, Box::new(move |_data, ctx| ctx.work.mark_dirty(node)));
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
        self.reads.buffer().push((signal, subscribe_derived::<D>));
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
            .ok_or_else(|| ReactError::NotWatched(self.data.entity))?;

        store
            .values
            .get(target.0)
            .map_err(ReactError::TargetEntityError)?
            .get_components::<D>()
            .map_err(ReactError::TargetQueryError)
    }
}
