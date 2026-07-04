//! A keyed reactive list: a signal of `Vec<I>` reconciled into an ordered set of
//! child entities on a host, one entity per key.
//!
//! [`ReactiveListExt::reactive_list`] takes three pieces:
//! - `items: FnMut() -> SignalResult<Vec<I>>` — the source collection, read
//!   reactively (any signal it reads via `.get()?` makes the list re-reconcile);
//! - `key: Fn(&I) -> K` — a stable identity per item;
//! - `child: Fn(&mut Commands, I) -> impl Bundle` — renders one item into a bundle
//!   (it may spawn its own per-item signals / child entities via `Commands`).
//!
//! It returns a [`ReactiveList`] **component**: drop it onto a host entity and the
//! host's children track the collection. The reconciliation is a [`spawn_effect`]
//! sink; on each source change a keyed diff yields additions (spawn + render +
//! attach), removals (despawn), retained-value **updates** (re-render on the same
//! entity — no respawn/reorder), and reorders. Order is enforced by splicing the
//! managed block into the host's relationship collection ([`enforce_order`]),
//! leaving static siblings in place. The bweb DOM reconciler follows `Children`
//! order automatically, so the list only has to keep `Children` correct.
//!
//! Teardown: replacing the [`ReactiveList`] (or despawning the host) despawns every
//! managed item and the reconciliation sink. This mirrors the reactive-insertion
//! teardown in [`super::insert`].

use bevy_ecs::{
    component::{Mutable, StorageType},
    lifecycle::{ComponentHook, HookContext},
    prelude::*,
    relationship::{Relationship, RelationshipTarget},
    world::DeferredWorld,
};
use bevy_platform::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use crate::cleanup::ReactiveCleanupExt;

use super::error::SignalResult;
use super::graph::{despawn_node, spawn_effect};

/// Spawns the reconciliation sink once the host entity is known. Consumed on the
/// first insert.
type ListBuilder = Box<dyn FnOnce(&mut Commands, Entity) -> Entity + Send + Sync>;

/// A keyed reactive list attached to a host entity. Produced by
/// [`ReactiveListExt::reactive_list`]; generic over the relationship `R` that
/// attaches items to the host (defaults to [`ChildOf`]).
pub struct ReactiveList<R = ChildOf> {
    /// Builds + spawns the reconciliation sink, given the host; run once by
    /// `on_insert`.
    builder: Mutex<Option<ListBuilder>>,
    /// Every entity the list currently manages, for teardown. Shared with the sink.
    managed: Arc<Mutex<HashSet<Entity>>>,
    /// The reconciliation sink node, recorded by `on_insert` for teardown.
    sink: Arc<Mutex<Option<Entity>>>,
    _marker: PhantomData<fn() -> R>,
}

impl<R: Send + Sync + 'static> Component for ReactiveList<R> {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;

    fn on_insert() -> Option<ComponentHook> {
        fn hook<R: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
            let host = ctx.entity;
            let builder = world
                .get::<ReactiveList<R>>(host)
                .and_then(|c| c.builder.lock().unwrap().take());
            let Some(builder) = builder else {
                return;
            };
            let sink_cell = world.get::<ReactiveList<R>>(host).unwrap().sink.clone();
            let mut commands = world.commands();
            let sink = builder(&mut commands, host);
            *sink_cell.lock().unwrap() = Some(sink);
        }
        Some(hook::<R>)
    }

    fn on_replace() -> Option<ComponentHook> {
        fn hook<R: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
            let host = ctx.entity;
            let (managed, sink) = {
                let c = world.get::<ReactiveList<R>>(host).unwrap();
                (c.managed.clone(), c.sink.lock().unwrap().take())
            };
            world.commands().queue(move |world: &mut World| {
                for entity in managed.lock().unwrap().drain() {
                    if let Ok(entity) = world.get_entity_mut(entity) {
                        entity.despawn();
                    }
                }
                if let Some(sink) = sink {
                    despawn_node(world, sink);
                }
            });
        }
        Some(hook::<R>)
    }
}

/// Per-key reconciliation state: the last-seen key order and each key's entity +
/// last value.
struct ListState<K, I> {
    order: Vec<K>,
    map: HashMap<K, (Entity, I)>,
}

impl<K, I> Default for ListState<K, I> {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            map: HashMap::default(),
        }
    }
}

/// The outcome of diffing a new collection against [`ListState`]. `additions` and
/// `updates` carry the item value so the child can render; `updates` are retained
/// keys whose value moved.
struct Diff<K, I> {
    additions: Vec<(K, I)>,
    updates: Vec<(K, I)>,
    removals: Vec<K>,
    order_changed: bool,
}

impl<K, I> ListState<K, I>
where
    K: Eq + Hash + Clone,
    I: PartialEq + Clone,
{
    fn reconcile(&mut self, new: Vec<I>, key: impl Fn(&I) -> K) -> Diff<K, I> {
        let new_keys: Vec<K> = new.iter().map(&key).collect();
        let new_set: HashSet<K> = new_keys.iter().cloned().collect();
        let old_set: HashSet<K> = self.map.keys().cloned().collect();

        let mut additions = Vec::new();
        let mut updates = Vec::new();
        for (k, item) in new_keys.iter().zip(new.iter()) {
            match self.map.get(k) {
                None => additions.push((k.clone(), item.clone())),
                Some((_, old)) if old != item => updates.push((k.clone(), item.clone())),
                Some(_) => {}
            }
        }

        let removals: Vec<K> = self
            .order
            .iter()
            .filter(|k| !new_set.contains(*k))
            .cloned()
            .collect();

        // Additions and removals leave the relative order of retained items
        // untouched, so order only changes when the retained subsequences disagree.
        let old_retained = self.order.iter().filter(|k| new_set.contains(*k));
        let new_retained = new_keys.iter().filter(|k| old_set.contains(*k));
        let order_changed = !old_retained.eq(new_retained);

        self.order = new_keys;

        Diff {
            additions,
            updates,
            removals,
            order_changed,
        }
    }
}

/// The `reactive_list` constructors, added to `Commands`.
pub trait ReactiveListExt {
    /// A keyed reactive list whose items are attached to the host as `ChildOf`
    /// children. See the [module docs](self).
    fn reactive_list<F, I, K, KF, CF, B>(
        &mut self,
        items: F,
        key: KF,
        child: CF,
    ) -> ReactiveList<ChildOf>
    where
        F: FnMut() -> SignalResult<Vec<I>> + Send + Sync + 'static,
        I: PartialEq + Clone + Send + Sync + 'static,
        K: Eq + Hash + Clone + Send + Sync + 'static,
        KF: Fn(&I) -> K + Send + Sync + 'static,
        CF: Fn(&mut Commands, I) -> B + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static;

    /// Like [`reactive_list`](Self::reactive_list), but attaches items through an
    /// arbitrary relationship `R`.
    fn reactive_list_related<F, I, K, KF, CF, B, R>(
        &mut self,
        items: F,
        key: KF,
        child: CF,
    ) -> ReactiveList<R>
    where
        F: FnMut() -> SignalResult<Vec<I>> + Send + Sync + 'static,
        I: PartialEq + Clone + Send + Sync + 'static,
        K: Eq + Hash + Clone + Send + Sync + 'static,
        KF: Fn(&I) -> K + Send + Sync + 'static,
        CF: Fn(&mut Commands, I) -> B + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
        R: Relationship;
}

impl ReactiveListExt for Commands<'_, '_> {
    fn reactive_list<F, I, K, KF, CF, B>(
        &mut self,
        items: F,
        key: KF,
        child: CF,
    ) -> ReactiveList<ChildOf>
    where
        F: FnMut() -> SignalResult<Vec<I>> + Send + Sync + 'static,
        I: PartialEq + Clone + Send + Sync + 'static,
        K: Eq + Hash + Clone + Send + Sync + 'static,
        KF: Fn(&I) -> K + Send + Sync + 'static,
        CF: Fn(&mut Commands, I) -> B + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
    {
        build_reactive_list(self, items, key, child)
    }

    fn reactive_list_related<F, I, K, KF, CF, B, R>(
        &mut self,
        items: F,
        key: KF,
        child: CF,
    ) -> ReactiveList<R>
    where
        F: FnMut() -> SignalResult<Vec<I>> + Send + Sync + 'static,
        I: PartialEq + Clone + Send + Sync + 'static,
        K: Eq + Hash + Clone + Send + Sync + 'static,
        KF: Fn(&I) -> K + Send + Sync + 'static,
        CF: Fn(&mut Commands, I) -> B + Send + Sync + 'static,
        B: Bundle + Send + Sync + 'static,
        R: Relationship,
    {
        build_reactive_list(self, items, key, child)
    }
}

fn build_reactive_list<F, I, K, KF, CF, B, R>(
    commands: &mut Commands,
    mut items: F,
    key: KF,
    child: CF,
) -> ReactiveList<R>
where
    F: FnMut() -> SignalResult<Vec<I>> + Send + Sync + 'static,
    I: PartialEq + Clone + Send + Sync + 'static,
    K: Eq + Hash + Clone + Send + Sync + 'static,
    KF: Fn(&I) -> K + Send + Sync + 'static,
    CF: Fn(&mut Commands, I) -> B + Send + Sync + 'static,
    B: Bundle + Send + Sync + 'static,
    R: Relationship,
{
    let _ = commands; // the sink is spawned later, once the host is known.
    let managed = Arc::new(Mutex::new(HashSet::default()));

    let builder: ListBuilder = {
        let managed = managed.clone();
        Box::new(move |commands: &mut Commands, host: Entity| {
            let mut state = ListState::<K, I>::default();
            spawn_effect(commands, move |commands: &mut Commands| {
                let Ok(new) = items() else {
                    // NotReady: leave the current list untouched.
                    return;
                };

                let diff = state.reconcile(new, &key);
                // Additions append to the relationship collection, so they (and
                // reorders) need the enforcement pass; removals keep survivor order.
                let needs_order = diff.order_changed || !diff.additions.is_empty();

                for k in &diff.removals {
                    if let Some((entity, _)) = state.map.remove(k) {
                        managed.lock().unwrap().remove(&entity);
                        commands.queue(move |world: &mut World| {
                            if let Ok(entity) = world.get_entity_mut(entity) {
                                entity.despawn();
                            }
                        });
                    }
                }

                for (k, item) in diff.additions {
                    let entity = commands.spawn_empty().id();
                    let bundle = child(&mut *commands, item.clone());
                    commands.entity(entity).insert((bundle, R::from(host)));
                    state.map.insert(k, (entity, item));
                    managed.lock().unwrap().insert(entity);
                }

                for (k, item) in diff.updates {
                    if let Some((entity, old)) = state.map.get_mut(&k) {
                        let entity = *entity;
                        let bundle = child(&mut *commands, item.clone());
                        // Re-render on the same entity: clean the prior bundle
                        // (subtree included) and re-insert. No respawn, no reorder.
                        commands
                            .entity(entity)
                            .reactive_cleanup::<B>()
                            .try_insert(bundle);
                        *old = item;
                    }
                }

                if needs_order {
                    let desired: Vec<Entity> = state
                        .order
                        .iter()
                        .filter_map(|k| state.map.get(k).map(|(entity, _)| *entity))
                        .collect();
                    commands.queue(enforce_order::<R>(host, desired));
                }
            })
        })
    };

    ReactiveList {
        builder: Mutex::new(Some(builder)),
        managed,
        sink: Arc::new(Mutex::new(None)),
        _marker: PhantomData,
    }
}

/// Rewrite `host`'s relationship collection so the list-managed entities appear in
/// `desired` order, spliced in at the block's current position. Entities outside the
/// list (e.g. static `children![..]` siblings) keep their places, and
/// `replace_related` skips relationship hooks for retained members, so only the
/// collection order (and its change tick) is touched.
fn enforce_order<R: Relationship>(
    host: Entity,
    desired: Vec<Entity>,
) -> impl FnOnce(&mut World) + Send + 'static {
    move |world: &mut World| {
        let desired: Vec<Entity> = desired
            .into_iter()
            .filter(|entity| world.get_entity(*entity).is_ok())
            .collect();

        let Ok(mut host_entity) = world.get_entity_mut(host) else {
            return;
        };
        let Some(current) = host_entity.get::<R::RelationshipTarget>() else {
            return;
        };
        let current: Vec<Entity> = RelationshipTarget::iter(current).collect();

        let members: HashSet<Entity> = desired.iter().copied().collect();
        let mut merged = Vec::with_capacity(current.len().max(desired.len()));
        let mut spliced = false;
        for entity in &current {
            if members.contains(entity) {
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
            host_entity.replace_related::<R>(&merged);
        }
    }
}
