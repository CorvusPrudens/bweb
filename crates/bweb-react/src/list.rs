use crate::{
    prelude::{Effect, Get, Read, ReadSignal, SignalExt, Target, Write, signal},
    signal::DerivedSignal,
};
use bevy_app::prelude::*;
use bevy_ecs::{
    lifecycle::HookContext,
    prelude::*,
    relationship::{Relationship, RelationshipTarget},
    system::SystemId,
    world::DeferredWorld,
};
use bevy_platform::collections::{HashMap, HashSet};
use core::marker::PhantomData;

pub(crate) struct ReactiveListPlugin;

impl Plugin for ReactiveListPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ListTargets>();
    }
}

#[derive(Component)]
#[component(on_insert = Self::on_insert_hook, on_replace = Self::on_replace_hook)]
pub struct ReactiveList<R = ChildOf> {
    target: Target,
    entities: ReadSignal<HashSet<Entity>>,
    effect: Effect,
    marker: core::marker::PhantomData<fn() -> R>,
}

impl<R: 'static> ReactiveList<R> {
    fn on_insert_hook(mut world: DeferredWorld, context: HookContext) {
        let value = world
            .get::<Self>(context.entity)
            .expect("entity should have `ReactiveList` component")
            .target;

        world
            .resource_mut::<ListTargets>()
            .0
            .insert(value, context.entity);
    }

    fn on_replace_hook(mut world: DeferredWorld, context: HookContext) {
        let value = world
            .get::<Self>(context.entity)
            .expect("entity should have `ReactiveList` component");

        let effect = value.effect.entity;
        let entities = value.entities.clone();
        let value = value.target;

        let mut commands = world.commands();

        commands.queue(move |world: &mut World| {
            for entity in entities.read().iter() {
                world.despawn(*entity);
            }
        });
        commands.entity(effect).despawn();

        world.resource_mut::<ListTargets>().0.remove(&value);
    }
}

struct ListState<I> {
    items: Vec<I>,
    set: HashSet<I>,
}

impl<I> Default for ListState<I> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            set: Default::default(),
        }
    }
}

impl<K> ListState<K>
where
    K: Eq + core::hash::Hash + Clone,
{
    pub fn diff(&mut self, mut new: Vec<K>) -> CollectionDiff<K> {
        let mut additions = Vec::new();
        let mut removals = Vec::new();

        core::mem::swap(&mut self.items, &mut new);

        let new_set: HashSet<K> = self.items.iter().cloned().collect();
        for (i, item) in self.items.iter().enumerate() {
            if !self.set.contains(item) {
                additions.push((i, item.clone()));
            }
        }

        // Additions and removals leave the relative order of retained
        // items untouched, so order only changes when the retained
        // subsequences disagree.
        let old_retained = new.iter().filter(|item| new_set.contains(*item));
        let new_retained = self.items.iter().filter(|item| self.set.contains(*item));
        let order_changed = !old_retained.eq(new_retained);

        for item in new.drain(..) {
            if !new_set.contains(&item) {
                removals.push(item);
            }
        }

        self.set = new_set;

        CollectionDiff {
            additions,
            removals,
            order_changed,
        }
    }
}

struct CollectionDiff<K> {
    pub additions: Vec<(usize, K)>,
    pub removals: Vec<K>,
    pub order_changed: bool,
}

#[derive(Resource, Default)]
struct ListTargets(HashMap<Target, Entity>);

// attempt to reduce monomorphization ig?
pub fn reactive_list<R, I, K, B>(
    commands: &mut Commands,
    collection: DerivedSignal<Vec<I>>,
    key: Box<dyn Fn(&I) -> K + Send + Sync>,
    child: SystemId<In<I>, B>,
) -> ReactiveList<R>
where
    R: Relationship,
    K: Eq + core::hash::Hash + Clone + Send + Sync + 'static,
    B: Bundle,
    I: Clone + Send + Sync + 'static,
{
    let target = Target::new();
    let mut state = ListState::default();
    let mut entities = HashMap::<K, Entity>::default();
    let mut queued_changes = Vec::new();
    let mut pending_order = false;

    // We memoize here so the list effect only
    // runs when our specific target changes.
    let target_sig =
        commands.memo(move |targets: Res<ListTargets>| targets.0.get(&target).copied());
    let (esig, set_esig) = signal(HashSet::new());

    let effect = commands.effect(move |mut commands: Commands| {
        let collection = collection.read();
        let new_keys = collection.iter().map(&key).collect();

        let CollectionDiff {
            additions,
            removals,
            order_changed,
        } = state.diff(new_keys);

        // Additions append to the relationship collection, so they need
        // the enforcement pass to land at their collection position.
        let needs_order = order_changed || !additions.is_empty();

        let mut esig = set_esig.write();
        for removal in removals {
            if let Some(entity) = entities.remove(&removal) {
                if let Ok(mut entity) = commands.get_entity(entity) {
                    entity.try_despawn();
                }
                esig.remove(&entity);
            }
        }

        for (i, key) in additions {
            let new_entity = commands.spawn_empty().id();
            entities.insert(key, new_entity);
            esig.insert(new_entity);

            // TODO: fix the part where items could have been removed before these
            // are applied.
            queued_changes.push((new_entity, collection[i].clone()));
        }

        match target_sig.get() {
            Some(target) => {
                for (new_entity, system_input) in queued_changes.drain(..) {
                    commands.queue(move |world: &mut World| -> Result {
                        let result = world.run_system_with(child, system_input)?;
                        world
                            .get_entity_mut(new_entity)?
                            .insert((result, R::from(target)));

                        Ok(())
                    });
                }

                if needs_order || pending_order {
                    pending_order = false;

                    let mut seen = HashSet::with_capacity(state.items.len());
                    let desired: Vec<Entity> = state
                        .items
                        .iter()
                        .filter(|key| seen.insert((*key).clone()))
                        .filter_map(|key| entities.get(key).copied())
                        .collect();

                    commands.queue(enforce_order::<R>(target, desired));
                }
            }
            None => pending_order |= needs_order,
        }
    });

    ReactiveList {
        target,
        entities: esig,
        effect,
        marker: PhantomData,
    }
}

/// Rewrite `target`'s relationship collection so the list-managed entities
/// appear in `desired` order, spliced in at the block's current position.
/// Entities outside the list (e.g. static `children![..]` siblings) keep
/// their places, and `replace_related` skips relationship hooks for
/// retained members, so only the collection order (and its change tick)
/// is touched.
fn enforce_order<R: Relationship>(
    target: Entity,
    desired: Vec<Entity>,
) -> impl FnOnce(&mut World) -> Result {
    move |world: &mut World| -> Result {
        let desired: Vec<Entity> = desired
            .into_iter()
            .filter(|entity| world.get_entity(*entity).is_ok())
            .collect();

        let Ok(mut target_entity) = world.get_entity_mut(target) else {
            return Ok(());
        };
        let Some(current) = target_entity.get::<R::RelationshipTarget>() else {
            return Ok(());
        };
        let current: Vec<Entity> = RelationshipTarget::iter(current).collect();

        let list_members: HashSet<Entity> = desired.iter().copied().collect();
        let mut merged = Vec::with_capacity(current.len().max(desired.len()));
        let mut spliced = false;
        for entity in &current {
            if list_members.contains(entity) {
                if !spliced {
                    merged.extend(desired.iter().copied());
                    spliced = true;
                }
            } else {
                merged.push(*entity);
            }
        }
        if !spliced {
            merged.extend(desired.iter().copied());
        }

        if merged != current {
            target_entity.replace_related::<R>(&merged);
        }

        Ok(())
    }
}
