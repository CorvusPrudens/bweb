use crate::{
    prelude::{Effect, Get, Read, ReadSignal, SignalExt, Target, Write, signal},
    signal::DerivedSignal,
};
use bevy_app::prelude::*;
use bevy_ecs::{
    lifecycle::HookContext, prelude::*, relationship::Relationship, system::SystemId,
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

        for item in new.drain(..) {
            if !new_set.contains(&item) {
                removals.push(item);
            }
        }

        self.set = new_set;

        CollectionDiff {
            additions,
            removals,
        }
    }
}

struct CollectionDiff<K> {
    pub additions: Vec<(usize, K)>,
    pub removals: Vec<K>,
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
        } = state.diff(new_keys);

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

        if let Some(target) = target_sig.get() {
            for (new_entity, system_input) in queued_changes.drain(..) {
                commands.queue(move |world: &mut World| -> Result {
                    let result = world.run_system_with(child, system_input)?;
                    world
                        .get_entity_mut(new_entity)?
                        .insert((result, R::from(target)));

                    Ok(())
                });
            }
        }
    });

    ReactiveList {
        target,
        entities: esig,
        effect,
        marker: PhantomData,
    }
}
