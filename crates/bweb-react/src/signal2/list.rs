//! Keyed lists: a collection signal projected onto a run of related entities.
//!
//! A list is an [`Effect`] with bookkeeping. It reads one signal whose value is
//! a slice, keys each element, and maintains one entity per key — spawning what
//! appeared, despawning what left, re-running the row body for an element whose
//! value changed, and splicing the survivors back into the container's
//! relationship collection in the order the slice gives.
//!
//! The reason it is a slice and not a `Vec` handed over by value is that the
//! collection never has to be cloned: [`ListSource`] borrows out of the
//! component the signal already points at. What does get cloned is a single
//! element, and only when that element is new or has changed — never the
//! collection, and never on a frame where nothing moved.

use core::{hash::Hash, marker::PhantomData};

use bevy_ecs::{
    lifecycle::HookContext,
    prelude::*,
    relationship::{Relationship, RelationshipTarget},
    world::DeferredWorld,
};
use bevy_platform::collections::{HashMap, HashSet};

use crate::signal2::{
    effect::Effect,
    source::{SignalStore, SourceSignal},
};

/// A component a list can be driven from: anything that can show its elements
/// as a contiguous slice.
///
/// The slice is the whole contract. A list re-reads it on every run and diffs
/// it against what it built last time, so the source is free to be a plain
/// `Vec` newtype, a relationship collection, or anything else that can hand out
/// a borrow.
pub trait ListSource {
    type Item;

    fn items(&self) -> &[Self::Item];
}

/// Every relationship collection backed by a `Vec<Entity>` is a list source,
/// which covers [`Children`] and the derived relationship targets alongside it.
///
/// This is a blanket rather than a macro because a manual impl downstream still
/// works: for a type local to the implementing crate, no upstream crate can add
/// the `RelationshipTarget` impl that would make the two overlap, so rustc
/// accepts both. The pair that genuinely *would* overlap — `Vec<T>` and
/// `[T; N]` — cannot be [`Component`]s, and so could never have been a list
/// source here anyway.
impl<T> ListSource for T
where
    T: RelationshipTarget<Collection = Vec<Entity>>,
{
    type Item = Entity;

    fn items(&self) -> &[Entity] {
        self.collection().as_slice()
    }
}

/// The container a controller builds rows for.
///
/// Read through a `Query` rather than captured in the effect's closure because
/// the closure is built by [`ReactiveList::new`], before the component has been
/// placed on anything and so before there is a container to capture.
#[derive(Component, Clone, Copy)]
pub(crate) struct ListOf(pub(crate) Entity);

/// A row's link back to the controller that spawned it.
///
/// The rows are also related to the *container* by `R`, but that link is about
/// ordering and says nothing about ownership — for a non-linked `R` it would
/// not even outlive the container. This one exists purely so that despawning
/// the controller takes the rows with it, which is the whole of a list's
/// teardown.
#[derive(Component)]
#[relationship(relationship_target = ListRows)]
pub(crate) struct RowOf(pub(crate) Entity);

/// The rows a controller currently owns. Maintained by the relationship hooks,
/// never written by hand.
#[derive(Component)]
#[relationship_target(relationship = RowOf, linked_spawn)]
pub(crate) struct ListRows(Vec<Entity>);

/// One live row.
struct Row<I> {
    entity: Entity,
    /// The item this row was last built from, kept as the baseline a change is
    /// measured against.
    ///
    /// This is the only clone a list makes, and it is made once per row per
    /// actual change rather than once per collection per frame.
    value: I,
}

/// What a list remembers between runs.
///
/// Lives in a `Local` on the effect's system. It cannot live on an entity: a
/// `Query<&mut ListState>` would conflict with the `Query<EntityRef>` inside
/// [`SignalStore`], which reads every component in the world.
struct ListState<K, I> {
    /// Keys in collection order as of the last run, deduplicated.
    keys: Vec<K>,
    rows: HashMap<K, Row<I>>,
}

impl<K, I> Default for ListState<K, I> {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            rows: HashMap::default(),
        }
    }
}

/// A keyed list of entities, maintained from a collection signal.
///
/// Place it on the entity the rows should be related to:
///
/// ```ignore
/// let tabs = commands.signal::<&Tabs>().watch(model);
/// let list: ReactiveList = ReactiveList::new(
///     tabs,
///     |tab| *tab,
///     move |tab, commands| tab_view(*tab, commands),
/// );
/// commands.entity(container).insert(list);
/// ```
///
/// `R` is the relationship the rows are attached by, [`ChildOf`] unless said
/// otherwise. Rows are spliced into the container's existing collection at the
/// position the list's block already occupies, so static siblings keep their
/// places.
///
/// A container holds one list per relationship type, this being a single
/// component: inserting a second `ReactiveList<R>` replaces the first and tears
/// its rows down. Two independent runs of rows under one parent want two lists
/// on two child containers.
///
/// The list stops when the component does — overwrite it, remove it, or despawn
/// the container, and the rows are despawned and the effect unregistered. As
/// with [`Effect`] there is deliberately no `Clone`: the component *is* the
/// list's lifetime.
#[derive(Component)]
#[component(
    on_insert = Self::on_insert,
    on_replace = Self::on_replace,
    clone_behavior = Ignore,
)]
pub struct ReactiveList<R = ChildOf> {
    /// Taken by `on_insert`, which parks it on a controller entity. `None` from
    /// then on.
    effect: Option<Effect>,
    /// Where that controller ended up, so `on_replace` can despawn it.
    controller: Option<Entity>,
    marker: PhantomData<fn() -> R>,
}

impl<R: Relationship> ReactiveList<R> {
    /// Build a list over `source`, identifying elements by `key` and building
    /// each row with `row`.
    ///
    /// `key` is what makes the list keyed rather than positional: two runs that
    /// produce the same key for an element are talking about the same row, so a
    /// reorder moves entities instead of respawning them. `row` is handed the
    /// element and a queue, so it can spawn a whole subtree and return the
    /// bundle for the row itself.
    #[must_use]
    pub fn new<S, K, F, G, B>(source: SourceSignal<&'static S>, key: F, row: G) -> Self
    where
        S: Component + ListSource,
        S::Item: Clone + PartialEq + Send + Sync + 'static,
        K: Eq + Hash + Clone + Send + Sync + 'static,
        F: Fn(&S::Item) -> K + Send + Sync + 'static,
        G: Fn(&S::Item, &mut Commands) -> B + Send + Sync + 'static,
        B: Bundle,
    {
        let body = move |In(controller): In<Entity>,
                         store: SignalStore,
                         lists: Query<&ListOf>,
                         mut state: Local<ListState<K, S::Item>>,
                         mut commands: Commands| {
            // Ensure we're subscribed to the source
            let Ok(value) = source.get(&store) else {
                return;
            };
            let Ok(&ListOf(container)) = lists.get(controller) else {
                return;
            };

            let items = value.items();

            // Key everything once, dropping duplicates
            let mut order: Vec<K> = Vec::with_capacity(items.len());
            let mut present: HashSet<K> = HashSet::with_capacity(items.len());
            let mut fresh: Vec<&S::Item> = Vec::with_capacity(items.len());
            for item in items {
                let k = key(item);
                if present.insert(k.clone()) {
                    order.push(k);
                    fresh.push(item);
                }
            }

            let state = &mut *state;

            // Removals first, so the order comparison below sees a row map that
            // holds exactly the retained keys.
            state.rows.retain(|k, row| {
                if present.contains(k) {
                    return true;
                }
                if let Ok(mut row_entity) = commands.get_entity(row.entity) {
                    row_entity.try_despawn();
                }
                false
            });

            // Additions and removals leave the relative order of the retained
            // rows untouched, so the order only really changed when the two
            // retained subsequences disagree.
            let order_changed = !state
                .keys
                .iter()
                .filter(|k| present.contains(*k))
                .eq(order.iter().filter(|k| state.rows.contains_key(*k)));

            let mut added = false;
            for (k, item) in order.iter().zip(fresh.iter().copied()) {
                let existing = state
                    .rows
                    .get(k)
                    .map(|row| (row.entity, row.value == *item));

                match existing {
                    Some((_, true)) => {}
                    Some((entity, false)) => {
                        // A retained row whose item changed is rebuilt in place:
                        // the entity, and so its position and anything else
                        // holding a reference to it, survives.
                        let bundle = row(item, &mut commands);
                        commands.entity(entity).insert(bundle);
                        if let Some(existing) = state.rows.get_mut(k) {
                            existing.value = item.clone();
                        }
                    }
                    None => {
                        let entity = commands.spawn_empty().id();
                        let bundle = row(item, &mut commands);
                        commands.entity(entity).insert((
                            bundle,
                            R::from(container),
                            RowOf(controller),
                        ));
                        state.rows.insert(
                            k.clone(),
                            Row {
                                entity,
                                value: item.clone(),
                            },
                        );
                        added = true;
                    }
                }
            }

            state.keys = order;

            // Relationship hooks generally place the new entity at the end,
            // so it still needs an ordering pass.
            if added || order_changed {
                let desired: Vec<Entity> = state
                    .keys
                    .iter()
                    .filter_map(|k| state.rows.get(k).map(|row| row.entity))
                    .collect();
                commands.queue(enforce_order::<R>(container, desired));
            }
        };

        Self {
            effect: Some(Effect::new(body)),
            controller: None,
            marker: PhantomData,
        }
    }
}

impl<R: 'static> ReactiveList<R> {
    /// Park the effect on a seperate controller entity.
    ///
    /// Since effects maintain their state on the entity their inserted in,
    /// other effects or lists would otherwise clash.
    fn on_insert(mut world: DeferredWorld, ctx: HookContext) {
        let entity = ctx.entity;

        let effect = world
            .get_mut::<Self>(entity)
            .and_then(|mut list| list.effect.take());

        let Some(effect) = effect else {
            log::warn!("reaective list on {entity} was inserted twice");
            return;
        };

        let controller = world.commands().spawn((effect, ListOf(entity))).id();

        if let Some(mut list) = world.get_mut::<Self>(entity) {
            list.controller = Some(controller);
        }
    }

    /// End the list when the component goes away.
    fn on_replace(mut world: DeferredWorld, ctx: HookContext) {
        let Some(controller) = world
            .get::<Self>(ctx.entity)
            .and_then(|list| list.controller)
        else {
            return;
        };

        world.commands().queue(move |world: &mut World| {
            if let Ok(controller) = world.get_entity_mut(controller) {
                controller.despawn();
            }
        });
    }
}

/// Rewrite `container`'s relationship collection so the list's rows appear in
/// `desired` order, spliced in at the block's current position.
///
/// Entities outside the list — static `children![..]` siblings, or anything
/// else that was parented here by hand — keep their places, and
/// `replace_related` skips the relationship hooks for retained members, so only
/// the collection's order and its change tick are touched.
fn enforce_order<R: Relationship>(
    container: Entity,
    desired: Vec<Entity>,
) -> impl FnOnce(&mut World) {
    move |world: &mut World| {
        // A row can be despawned between the effect queueing this and the
        // command running — by the row body itself, or by a despawn of the
        // container that took its children.
        let desired: Vec<Entity> = desired
            .into_iter()
            .filter(|entity| world.get_entity(*entity).is_ok())
            .collect();

        let Ok(mut container) = world.get_entity_mut(container) else {
            return;
        };
        let Some(current) = container.get::<R::RelationshipTarget>() else {
            return;
        };
        let current: Vec<Entity> = RelationshipTarget::iter(current).collect();

        let members: HashSet<Entity> = desired.iter().copied().collect();
        let mut merged = Vec::with_capacity(current.len().max(desired.len()));
        let mut spliced = false;
        for entity in &current {
            if members.contains(entity) {
                // The first row we meet is where the whole block goes.
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

        // The guard, not an optimization: writing the collection back marks it
        // changed, and a list whose own source is that collection would then
        // wake itself forever.
        if merged != current {
            container.replace_related::<R>(&merged);
        }
    }
}
