use bevy_app::prelude::*;
use bevy_ecs::{
    prelude::*,
    query::{QueryData, ROQueryItem, ReadOnlyQueryData},
    relationship::Relationship,
    schedule::ScheduleLabel,
};

#[cfg(feature = "dev")]
use bevy_platform::collections::HashSet;

#[cfg(feature = "dev")]
use bevy_ecs::change_detection::MaybeLocation;

use crate::prelude::SQuery;

pub mod any;
pub mod cleanup;
pub mod effect;
pub mod list;
pub mod optional;
pub mod signal;
pub mod signal2;
pub mod target;

pub struct ReactPlugin;

impl Plugin for ReactPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Reactions>()
            .init_schedule(ReactSchedule)
            .configure_sets(PostUpdate, ReactSystems::EvaluateReactions)
            .configure_sets(
                ReactSchedule,
                (
                    ReactScheduleSystems::EvaluateSignals,
                    ReactScheduleSystems::EvaluateEffects
                        .after(ReactScheduleSystems::EvaluateSignals),
                    ReactScheduleSystems::PropagateChanges
                        .after(ReactScheduleSystems::EvaluateEffects),
                ),
            )
            .add_plugins((
                signal::SignalPlugin,
                signal2::Signal2Plugin,
                target::TargetPlugin,
                cleanup::CleanupPlugin,
                effect::EffectPlugin,
                list::ReactiveListPlugin,
            ))
            .add_systems(
                PostUpdate,
                evaluate_reactions.in_set(ReactSystems::EvaluateReactions),
            );
    }
}

#[derive(Resource)]
pub struct Reactions {
    reaction_limit: usize,
    count: usize,
    #[cfg(feature = "dev")]
    locations: HashSet<MaybeLocation>,
}

impl Default for Reactions {
    fn default() -> Self {
        Self::new(16)
    }
}

impl Reactions {
    pub fn new(reaction_limit: usize) -> Self {
        Self {
            reaction_limit,
            count: 0,
            #[cfg(feature = "dev")]
            locations: Default::default(),
        }
    }

    #[cfg_attr(feature = "dev", track_caller)]
    pub fn increment(&mut self) {
        self.count += 1;
        #[cfg(feature = "dev")]
        self.locations.insert(MaybeLocation::caller());
    }

    fn clear(&mut self) {
        self.count = 0;
        #[cfg(feature = "dev")]
        self.locations.clear();
    }
}

fn evaluate_reactions(world: &mut World) {
    world.schedule_scope(ReactSchedule, |world, schedule| {
        #[cfg(feature = "dev")]
        let start = bevy_platform::time::Instant::now();
        let mut total = 0;
        let reaction_limit = world.resource::<Reactions>().reaction_limit;

        #[cfg(feature = "dev")]
        #[derive(Default)]
        struct ReactionReport {
            locations: HashSet<MaybeLocation>,
            counts: Vec<usize>,
        }

        #[cfg(feature = "dev")]
        let mut report = ReactionReport::default();

        for _ in 0..reaction_limit {
            world.resource_mut::<Reactions>().clear();

            schedule.run(world);

            total += 1;
            let reactions = world.resource::<Reactions>();
            let new_reactions = reactions.count;

            #[cfg(feature = "dev")]
            {
                report.locations.extend(reactions.locations.iter().cloned());
                report.counts.push(new_reactions);
            }

            if new_reactions == 0 {
                break;
            }
        }

        #[cfg(feature = "dev")]
        {
            if !matches!(report.counts.as_slice(), &[0]) {
                let elapsed = start.elapsed();
                log::debug!("old ReactSchedule settled in {elapsed:?} over {total} pass(es)");

                let mut locations = report
                    .locations
                    .iter()
                    .map(|l| format!("{l}"))
                    .collect::<Vec<_>>();
                locations.sort_unstable();

                log::trace!("Reactive evaluation report:");
                log::trace!(
                    "Reaction counts ({}): {:?}",
                    report.counts.len(),
                    report.counts
                );
                log::trace!("Locations: {locations:#?}");
            }
        }

        if total == reaction_limit {
            log::warn!("Reached reactive evaluation limit");
        }
    });
}

/// Live reactive-node counts as `(old_framework, signal2)`. Every readable node
/// in either framework carries a `SignalGc`, so this is the live-node census used
/// to track the push-reactivity migration: as object reactivity moves off the old
/// `signal/` stack onto `signal2`, the first count should fall and the second rise.
/// Cheap enough for tests and `dev` diagnostics; not intended for hot paths.
pub fn reactive_node_counts(world: &mut World) -> (usize, usize) {
    (
        signal::live_node_count(world),
        signal2::live_node_count(world),
    )
}

#[derive(ScheduleLabel, PartialEq, Eq, Clone, Debug, Hash)]
pub struct ReactSchedule;

#[derive(SystemSet, PartialEq, Eq, Clone, Debug, Hash)]
pub enum ReactSystems {
    EvaluateReactions,
}

#[derive(SystemSet, PartialEq, Eq, Clone, Debug, Hash)]
pub enum ReactScheduleSystems {
    EvaluateSignals,
    EvaluateEffects,
    PropagateChanges,
}

pub trait SignalExt {
    #[must_use]
    fn query<D>(
        &mut self,
        target: impl Into<target::EntityTarget>,
    ) -> signal::DerivedSignal<Option<<D as QueryClone>::Output>>
    where
        D: QueryClone + ReadOnlyQueryData + 'static,
        <D as QueryClone>::Output: Clone + Send + Sync + 'static,
    {
        let target: target::EntityTarget = target.into();
        self.derive(move |query: SQuery<D>| {
            query
                .get(target)
                .ok()
                .map(|v| <D as QueryClone>::tuple_clone(v))
        })
    }

    #[must_use]
    fn derive<S, O, M>(&mut self, system: S) -> signal::DerivedSignal<O>
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: Clone + Send + Sync + 'static;

    #[must_use]
    fn memo<S, O, M>(&mut self, system: S) -> signal::DerivedSignal<O>
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: PartialEq + Clone + Send + Sync + 'static;

    fn has<C: Component>(&mut self, target: Entity) -> prelude::ReadSignal<bool>;

    #[must_use]
    fn effect<S, M>(&mut self, system: S) -> effect::Effect
    where
        S: IntoSystem<(), (), M> + Send + Sync + 'static,
        M: 'static;

    /// Derive a trailing-edge debounced view of `source`: the returned signal
    /// adopts `source`'s latest value only after `source` has been quiet for
    /// `delay`. Rapid bursts collapse into a single trailing update.
    #[must_use]
    fn debounce<T>(
        &mut self,
        source: signal::DerivedSignal<T>,
        delay: core::time::Duration,
    ) -> signal::DerivedSignal<T>
    where
        T: Clone + Default + Send + Sync + 'static;

    #[must_use]
    fn derive_list<S1, M1, F, I, K, S2, O2, M2, R>(
        &mut self,
        it: S1,
        key: F,
        child: S2,
    ) -> list::ReactiveList<R>
    where
        S1: IntoSystem<(), Vec<I>, M1> + Send + Sync + 'static,
        S1::System: ReadOnlySystem,
        M1: 'static,
        F: Fn(&I) -> K + Send + Sync + 'static,
        I: PartialEq + Clone + Send + Sync + 'static,
        K: Eq + core::hash::Hash + Clone + Send + Sync + 'static,
        S2: IntoSystem<In<I>, O2, M2> + 'static,
        O2: Bundle,
        R: Relationship;
}

impl SignalExt for Commands<'_, '_> {
    fn derive<S, O, M>(&mut self, system: S) -> signal::DerivedSignal<O>
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: Clone + Send + Sync + 'static,
    {
        signal::DerivedSignal::new(self.reborrow(), system)
    }

    fn memo<S, O, M>(&mut self, system: S) -> signal::DerivedSignal<O>
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: PartialEq + Clone + Send + Sync + 'static,
    {
        signal::DerivedSignal::memo(self.reborrow(), system)
    }

    fn has<C: Component>(&mut self, target: Entity) -> prelude::ReadSignal<bool> {
        use crate::prelude::*;
        let (has, set_has) = signal(false);

        self.queue({
            let set_has = set_has.clone();
            move |world: &mut World| {
                if let Ok(entity) = world.get_entity(target)
                    && entity.contains::<C>()
                {
                    set_has.set(true);
                }
            }
        });

        self.spawn(
            Observer::new({
                let set_has = set_has.clone();
                move |_: On<Add, C>| {
                    set_has.set(true);
                }
            })
            .with_entity(target),
        );
        self.spawn(
            Observer::new({
                let set_has = set_has.clone();
                move |_: On<Remove, C>| {
                    set_has.set(false);
                }
            })
            .with_entity(target),
        );

        has
    }

    fn effect<S, M>(&mut self, system: S) -> effect::Effect
    where
        S: IntoSystem<(), (), M> + Send + Sync + 'static,
        M: 'static,
    {
        effect::Effect::new(system, self.reborrow())
    }

    fn debounce<T>(
        &mut self,
        source: signal::DerivedSignal<T>,
        delay: core::time::Duration,
    ) -> signal::DerivedSignal<T>
    where
        T: Clone + Default + Send + Sync + 'static,
    {
        #[cfg(feature = "web")]
        {
            use crate::prelude::*;
            use bevy_platform::sync::{
                Arc,
                atomic::{AtomicU32, Ordering},
            };

            let (get, set) = crate::prelude::signal(T::default());
            let derived = self.derive(move || get.get());

            let epoch = Arc::new(AtomicU32::new(0));
            let effect = self.effect(move || {
                let value = source.get();
                let generation = epoch.fetch_add(1, Ordering::Relaxed) + 1;
                let (epoch, write, value) = (epoch.clone(), set.clone(), value.clone());

                let set = set.clone();
                bweb::task::spawn_local(async move |mut world: bweb::prelude::TaskWorld| {
                    bweb::time::sleep(delay).await;
                    if epoch.load(Ordering::Relaxed) == generation {
                        world.with(|_| set.set(value));
                    }
                });
            });

            self.entity(derived.entity()).with_child(effect);

            derived
        }
        #[cfg(not(feature = "web"))]
        {
            todo!("Debounced signals are not implemented on this platform")
        }
    }

    fn derive_list<S1, M1, F, I, K, S2, O2, M2, R>(
        &mut self,
        it: S1,
        key: F,
        child: S2,
    ) -> list::ReactiveList<R>
    where
        S1: IntoSystem<(), Vec<I>, M1> + Send + Sync + 'static,
        S1::System: ReadOnlySystem,
        M1: 'static,
        F: Fn(&I) -> K + Send + Sync + 'static,
        I: PartialEq + Clone + Send + Sync + 'static,
        K: Eq + core::hash::Hash + Clone + Send + Sync + 'static,
        S2: IntoSystem<In<I>, O2, M2> + 'static,
        O2: Bundle,
        R: Relationship,
    {
        let collection = self.derive(it);
        let key = Box::new(key) as Box<dyn Fn(&I) -> K + Send + Sync>;
        let child = self.register_system(child);

        list::reactive_list(self, collection, key, child)
    }
}

pub trait QueryClone: QueryData {
    type Output;

    fn tuple_clone(this: ROQueryItem<'_, '_, Self>) -> Self::Output;
}

impl<T: Component + Clone> QueryClone for &T {
    type Output = T;

    fn tuple_clone(this: ROQueryItem<'_, '_, Self>) -> Self::Output {
        this.clone()
    }
}

macro_rules! tuple_clone {
    ($($ty:ident),*) => {
        impl<$($ty: Component + Clone),*> QueryClone for ($(&$ty,)*) {
            type Output = ($($ty,)*);

            #[allow(non_snake_case, clippy::unused_unit)]
            fn tuple_clone(this: ROQueryItem<'_, '_, Self>) -> Self::Output {
                let ($($ty,)*) = this;
                ($($ty.clone(),)*)
            }
        }
    };
}

variadics_please::all_tuples!(tuple_clone, 0, 15, T);

pub mod prelude {
    pub use crate::any::{AnyBundle, IntoAnyBundle};
    pub use crate::effect::Effect;
    pub use crate::list::ReactiveList;
    pub use crate::optional::{IntoOptionalBundle, OptionalBundle};
    pub use crate::signal::{
        DerivedSignal, MappedSignal, OptionSignal, Signal,
        rw_signal::{ReadSignal, RwSignal, WriteSignal, signal},
        signal_query::SQuery,
        signal_res::SRes,
        traits::*,
    };
    pub use crate::target::{EntityTarget, Target, TargetQueryError, Targets};
    pub use crate::{ReactPlugin, SignalExt};
}

#[cfg(test)]
mod test {
    use crate::prelude::*;
    use bevy_app::prelude::*;
    use bevy_ecs::{prelude::*, system::RunSystemOnce};

    #[derive(Component, PartialEq, Clone)]
    struct TestData(pub f32);

    #[derive(Resource, PartialEq, Clone)]
    struct TestRes(pub usize);

    #[test]
    fn test_memo() {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        world.insert_resource(TestRes(0));
        let test_entity = world.spawn(TestData(1.0)).id();

        let mut commands = world.commands();
        let test_sig =
            commands.memo(move |q: Query<&TestData>| q.get(test_entity).unwrap().clone());

        commands.effect(move |mut res: ResMut<TestRes>| {
            let _test_sig = test_sig.get();
            res.0 += 1;
        });

        app.update();
        let world = app.world_mut();

        assert_eq!(world.resource::<TestRes>().0, 1);

        world.get_mut::<TestData>(test_entity).unwrap().0 = 2.0;

        app.update();
        let world = app.world_mut();
        assert_eq!(world.resource::<TestRes>().0, 2);

        app.update();
        let world = app.world_mut();
        assert_eq!(world.resource::<TestRes>().0, 2);
    }

    #[test]
    fn test_effect() {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        world.insert_resource(TestRes(0));
        let test_entity = world.spawn(TestData(1.0)).id();

        let mut commands = world.commands();
        let test_sig = commands.query::<&TestData>(test_entity);

        commands.effect(move |mut res: ResMut<TestRes>| {
            let _test_sig = test_sig.get();
            res.0 += 1;
        });

        app.update();
        let world = app.world_mut();

        assert_eq!(world.resource::<TestRes>().0, 1);

        world.get_mut::<TestData>(test_entity).unwrap().0 = 2.0;

        app.update();
        let world = app.world_mut();
        assert_eq!(world.resource::<TestRes>().0, 2);

        app.update();
        let world = app.world_mut();
        assert_eq!(world.resource::<TestRes>().0, 2);
    }

    #[derive(Resource, Clone)]
    struct Items(Vec<u32>);

    /// The container's children in `Children` order, as their item keys.
    fn list_keys(world: &mut World, container: Entity) -> Vec<u32> {
        world
            .get::<Children>(container)
            .map(|c| c.iter().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| world.get::<TestData>(e).map(|d| d.0 as u32))
            .collect()
    }

    fn items_app(initial: Vec<u32>) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        world.insert_resource(Items(initial));

        let mut commands = world.commands();
        let list: ReactiveList = commands.derive_list(
            |items: SRes<Items>| items.0.clone(),
            |i| *i,
            |key: In<u32>| TestData(key.0 as f32),
        );
        let container = commands.spawn(list).id();

        app.update();
        (app, container)
    }

    #[test]
    fn test_reactive_list_reorders() {
        let (mut app, container) = items_app(vec![1, 2, 3]);
        let world = app.world_mut();
        assert_eq!(list_keys(world, container), vec![1, 2, 3]);

        let before: Vec<Entity> = world.get::<Children>(container).unwrap().iter().collect();

        world.resource_mut::<Items>().0 = vec![3, 1, 2];
        app.update();
        let world = app.world_mut();
        assert_eq!(list_keys(world, container), vec![3, 1, 2]);

        // A reorder moves the existing entities; it doesn't respawn them.
        let mut after: Vec<Entity> = world.get::<Children>(container).unwrap().iter().collect();
        let mut sorted_before = before;
        sorted_before.sort();
        after.sort();
        assert_eq!(sorted_before, after);
    }

    #[test]
    fn test_reactive_list_inserts_in_position() {
        let (mut app, container) = items_app(vec![1, 4]);
        let world = app.world_mut();
        assert_eq!(list_keys(world, container), vec![1, 4]);

        world.resource_mut::<Items>().0 = vec![1, 2, 3, 4];
        app.update();
        let world = app.world_mut();
        assert_eq!(list_keys(world, container), vec![1, 2, 3, 4]);

        // Removal keeps the survivors' order.
        world.resource_mut::<Items>().0 = vec![2, 4];
        app.update();
        let world = app.world_mut();
        assert_eq!(list_keys(world, container), vec![2, 4]);
    }

    #[derive(Resource, Clone)]
    struct Pairs(Vec<(u32, u32)>);

    #[test]
    fn test_reactive_list_updates_retained_items() {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        world.insert_resource(Pairs(vec![(1, 10), (2, 20)]));

        let mut commands = world.commands();
        let list: ReactiveList = commands.derive_list(
            |items: SRes<Pairs>| items.0.clone(),
            |(key, _)| *key,
            |item: In<(u32, u32)>| TestData(item.0.1 as f32),
        );
        let container = commands.spawn(list).id();
        app.update();

        let world = app.world_mut();
        assert_eq!(list_keys(world, container), vec![10, 20]);
        let before: Vec<Entity> = world.get::<Children>(container).unwrap().iter().collect();

        // A retained key whose item value changed re-runs the child on the
        // existing entity instead of respawning it.
        world.resource_mut::<Pairs>().0 = vec![(1, 11), (2, 20)];
        app.update();
        let world = app.world_mut();
        assert_eq!(list_keys(world, container), vec![11, 20]);

        let after: Vec<Entity> = world.get::<Children>(container).unwrap().iter().collect();
        assert_eq!(
            before, after,
            "updates must not respawn or reorder entities"
        );
    }

    #[test]
    fn test_reactive_list_preserves_static_siblings() {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        world.insert_resource(Items(vec![1, 2]));

        let container = world.spawn_empty().id();
        // A static child that isn't managed by the list.
        let static_child = world.spawn((ChildOf(container), TestData(99.0))).id();

        let mut commands = world.commands();
        let list: ReactiveList = commands.derive_list(
            |items: SRes<Items>| items.0.clone(),
            |i| *i,
            |key: In<u32>| TestData(key.0 as f32),
        );
        commands.entity(container).insert(list);

        app.update();
        let world = app.world_mut();
        assert_eq!(list_keys(world, container), vec![99, 1, 2]);

        world.resource_mut::<Items>().0 = vec![2, 1];
        app.update();
        let world = app.world_mut();

        assert_eq!(list_keys(world, container), vec![99, 2, 1]);
        let first = world
            .get::<Children>(container)
            .unwrap()
            .iter()
            .next()
            .unwrap();
        assert_eq!(first, static_child);
    }

    #[test]
    fn test_reactive_list() {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        world.insert_resource(TestRes(0));
        let test_entity = world.spawn(TestData(1.0)).id();

        let mut commands = world.commands();

        let list: ReactiveList = commands.derive_list(
            move |q: SQuery<&TestData>| {
                (0..(q.get(test_entity).unwrap().0 as usize)).collect::<Vec<_>>()
            },
            |i| *i,
            |index: In<usize>| TestData(index.0 as f32),
        );

        commands.spawn(list);

        app.update();
        let world = app.world_mut();

        assert_eq!(
            world
                .run_system_once(|data: Query<&TestData>| data.iter().len())
                .unwrap(),
            2
        );

        world.get_mut::<TestData>(test_entity).unwrap().0 = 3.0;

        app.update();
        let world = app.world_mut();

        assert_eq!(
            world
                .run_system_once(|data: Query<&TestData>| data.iter().len())
                .unwrap(),
            4
        );
    }
}
