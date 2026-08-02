//! Waking subscribers for a change the scan structurally cannot see.
//!
//! Every other input to this graph is a change tick. A removal has none: once
//! `T` is off the entity there is no `Changed<T>` to match and no row for the
//! scan to visit, so a subscriber over [`Option<&T>`] would keep reporting the
//! last value `T` ever had.
//!
//! So removals come in through an observer instead. The first signal built over
//! `Option<&T>` spawns one global `On<Remove, T>` observer, which does no work
//! beyond pushing the entity into [`RemovedSources`]. The settle loop drains
//! that at the top of each pass and dispatches, which is what puts the wakeup in
//! the same pass as everything else — a removal and a change in the same frame
//! settle together rather than one frame apart.
//!
//! # Why the dispatch is deferred rather than done in the observer
//!
//! Two reasons, and the first is correctness: `Remove` fires *before* the
//! component leaves the entity, so a dispatch from inside the observer would
//! hand subscribers the value that is about to disappear. The second is that a
//! derived node subscribes by marking rather than by running, and a mark is only
//! worth anything if it lands in a live [`ReactiveWork`] — the observer has none.

use bevy_ecs::{prelude::*, world::CommandQueue};
use core::any::TypeId;

use bevy_platform::collections::HashSet;
use smallvec::SmallVec;

use crate::signal2::{
    ReactiveWork, SignalData, SubscriberContext, SubscriberSet, mapped::run_registered_mapper,
};

/// Dispatches every subscriber a source entity carries for one signal type.
///
/// A plain fn pointer because [`RemovedSources`] holds removals for every signal
/// type at once and cannot be generic over any of them.
type RemovalDispatch = fn(&mut World, Entity, &mut ReactiveWork);

/// Entities that lost a watched component since the last pass.
///
/// Ordinary `Vec` rather than a set: a removal is rare, the list is drained
/// every pass, and a duplicate only costs one extra dispatch of an idempotent
/// insert.
#[derive(Resource, Default)]
pub(crate) struct RemovedSources(Vec<(Entity, RemovalDispatch)>);

/// Component types that already have a removal observer, per signal type that
/// asked for one.
///
/// Keyed by both because the same component can appear in more than one signal
/// — `Option<&Collapse>` and `(Option<&Collapse>, &Width)` each need their own
/// dispatch, and each needs it exactly once.
#[derive(Resource, Default)]
pub(crate) struct WatchedRemovals(HashSet<(TypeId, TypeId)>);

/// Spawn the `On<Remove, T>` observer that wakes `S`'s subscribers, unless one
/// is already watching that pair.
pub(crate) fn watch_removals<T, S>(commands: &mut Commands)
where
    T: Component,
    S: SignalData,
    for<'w, 's> S::Item<'w, 's>: Copy,
{
    commands.queue(|world: &mut World| {
        let key = (TypeId::of::<T>(), TypeId::of::<S>());
        if !world
            .get_resource_or_init::<WatchedRemovals>()
            .0
            .insert(key)
        {
            return;
        }

        world.spawn(Observer::new(
            // Global, so it sees every removal of `T` anywhere — including the
            // overwhelming majority on entities no signal watches. The filter
            // is what keeps those to an archetype check: no push, and no
            // fruitless lookup when the pass drains.
            |on: On<Remove, T>,
             watched: Query<(), With<SubscriberSet<S>>>,
             mut removed: ResMut<RemovedSources>| {
                if watched.get(on.entity).is_ok() {
                    removed.0.push((on.entity, dispatch_all::<S>));
                }
            },
        ));
    });
}

/// Dispatch every removal recorded since the last pass.
///
/// Runs at the top of a settle pass, beside
/// [`drain_dirty_cells`](crate::signal2::cell::drain_dirty_cells) and for the
/// same reason: what it produces belongs to this pass's dirty list, not the
/// next one's.
pub(crate) fn drain_removed_sources(world: &mut World, work: &mut ReactiveWork) {
    if world.resource::<RemovedSources>().0.is_empty() {
        return;
    }

    // Taken rather than drained in place: a dispatch can remove another
    // component, and the borrow cannot be held across the `&mut World`.
    let removed = core::mem::take(&mut world.resource_mut::<RemovedSources>().0);
    for (entity, dispatch) in removed {
        dispatch(world, entity, work);
    }
}

/// Run every subscriber `source` carries for `D`, against its current value.
///
/// The scan's dispatch loop, lifted out for a caller that has to run it for one
/// entity outside a scan. Unlike
/// [`dispatch_once`](crate::signal2::mapped::dispatch_once) — which serves a
/// single freshly-attached subscriber and can afford to throw its
/// [`ReactiveWork`] away — this takes the pass's own, so a derived node's mark
/// survives.
fn dispatch_all<D>(world: &mut World, source: Entity, work: &mut ReactiveWork)
where
    D: SignalData,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    let mut queue = CommandQueue::default();
    // Collected rather than run in place: a mapper system needs `&mut World`,
    // which is exactly what the borrow of the subscriber set rules out.
    let mut systems: SmallVec<[(Entity, Entity); 1]> = SmallVec::new();

    {
        // The entity can be gone entirely — `Remove` also fires on despawn,
        // which takes the subscriber set with it.
        let Ok(entity) = world.get_entity(source) else {
            return;
        };
        let Ok(item) = entity.get_components::<D>() else {
            return;
        };
        let Some(subs) = world.get::<SubscriberSet<D>>(source) else {
            return;
        };

        let mut commands = Commands::new(&mut queue, world);
        for closure in &subs.closures.f {
            work.dispatched += 1;
            closure(
                item,
                SubscriberContext {
                    world,
                    work: &mut *work,
                    commands: commands.reborrow(),
                },
            );
        }

        systems.extend(
            subs.systems
                .f
                .iter()
                .zip(&subs.systems.subs)
                .map(|(system, sub)| (*system, sub.owner)),
        );
    }
    queue.apply(world);

    for (system, owner) in systems {
        work.dispatched += 1;
        run_registered_mapper::<D>(world, system, owner, source);
    }
}
