use bevy_app::prelude::*;
use bevy_ecs::{
    change_detection::MaybeLocation,
    prelude::*,
    query::{QueryData, ROQueryItem, ReadOnlyQueryData},
    relationship::Relationship,
    schedule::ScheduleLabel,
};

#[cfg(debug_assertions)]
use bevy_platform::collections::HashSet;

use crate::prelude::SQuery;

pub mod any;
pub mod cleanup;
pub mod effect;
pub mod list;
pub mod optional;
pub mod signal;
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
    #[cfg(debug_assertions)]
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
            #[cfg(debug_assertions)]
            locations: Default::default(),
        }
    }

    #[cfg_attr(debug_assertions, track_caller)]
    pub fn increment(&mut self) {
        self.count += 1;
        #[cfg(debug_assertions)]
        self.locations.insert(MaybeLocation::caller());
    }

    fn clear(&mut self) {
        self.count = 0;
        #[cfg(debug_assertions)]
        self.locations.clear();
    }
}

fn evaluate_reactions(world: &mut World) {
    world.schedule_scope(ReactSchedule, |world, schedule| {
        let mut total = 0;
        let reaction_limit = world.resource::<Reactions>().reaction_limit;

        #[cfg(debug_assertions)]
        #[derive(Default)]
        struct ReactionReport {
            locations: HashSet<MaybeLocation>,
            counts: Vec<usize>,
        }

        #[cfg(debug_assertions)]
        let mut report = ReactionReport::default();

        for _ in 0..reaction_limit {
            world.resource_mut::<Reactions>().clear();

            schedule.run(world);

            total += 1;
            let reactions = world.resource::<Reactions>();
            let new_reactions = reactions.count;

            #[cfg(debug_assertions)]
            {
                report.locations.extend(reactions.locations.iter().cloned());
                report.counts.push(new_reactions);
            }

            if new_reactions == 0 {
                break;
            }
        }

        #[cfg(debug_assertions)]
        {
            if !matches!(report.counts.as_slice(), &[0]) {
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
    fn query<D>(
        &mut self,
        target: impl target::EntityTarget + Clone + Send + Sync + 'static,
    ) -> signal::DerivedSignal<Option<<D as QueryClone>::Output>>
    where
        D: QueryClone + ReadOnlyQueryData + 'static,
        <D as QueryClone>::Output: Clone + Send + Sync + 'static,
    {
        self.derive(move |query: SQuery<D>| {
            query
                .get(target.clone())
                .ok()
                .map(|v| <D as QueryClone>::tuple_clone(v))
        })
    }

    fn derive<S, O, M>(&mut self, system: S) -> signal::DerivedSignal<O>
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: Clone + Send + Sync + 'static;

    fn memo<S, O, M>(&mut self, system: S) -> signal::DerivedSignal<O>
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
        O: PartialEq + Clone + Send + Sync + 'static;

    fn has<C: Component>(&mut self, target: Entity) -> prelude::ReadSignal<bool>;

    fn effect<S, M>(&mut self, system: S) -> effect::Effect
    where
        S: IntoSystem<(), (), M> + Send + Sync + 'static,
        M: 'static;

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
