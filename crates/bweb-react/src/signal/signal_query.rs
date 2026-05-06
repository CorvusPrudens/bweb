use crate::target::{EntityTarget, Targets};
use bevy_ecs::{
    component::{ComponentId, Components},
    prelude::*,
    query::{QueryData, QueryEntityError, QueryFilter, ROQueryItem, ReadOnlyQueryData},
    system::{ReadOnlySystemParam, SystemParam},
    world::unsafe_world_cell::UnsafeWorldCell,
};

/// A signal query, equivalent to [`Query`] with additional reactive tracking.
pub struct SQuery<'w, 's, D: QueryData, F: QueryFilter = ()> {
    query: Query<'w, 's, D, F>,
    targets: Res<'w, Targets>,
    components: &'s [ComponentId],
}

unsafe impl<D: QueryData + 'static, F: QueryFilter + 'static> SystemParam for SQuery<'_, '_, D, F> {
    type State = (
        QueryState<D, F>,
        <Res<'static, Targets> as SystemParam>::State,
        Vec<ComponentId>,
    );
    type Item<'world, 'state> = SQuery<'world, 'state, D, F>;

    fn init_state(world: &mut World) -> Self::State {
        let query_state = Query::<'_, '_, D, F>::init_state(world);
        let target_state = Res::<'_, Targets>::init_state(world);
        let component_state = match query_state
            .component_access()
            .access()
            .try_iter_component_access()
        {
            Ok(access) => access.map(|c| *c.index()).collect(),
            // TODO: we might need to do something better here
            Err(e) => {
                panic!("failed to track query: {e}");
            }
        };

        (query_state, target_state, component_state)
    }

    fn init_access(
        state: &Self::State,
        system_meta: &mut bevy_ecs::system::SystemMeta,
        component_access_set: &mut bevy_ecs::query::FilteredAccessSet,
        world: &mut World,
    ) {
        Query::<'_, '_, D, F>::init_access(&state.0, system_meta, component_access_set, world);
        Res::<'_, Targets>::init_access(&state.1, system_meta, component_access_set, world);
        <&'_ Components>::init_access(&(), system_meta, component_access_set, world);
    }

    unsafe fn validate_param<'w, 's>(
        state: &mut Self::State,
        system_meta: &bevy_ecs::system::SystemMeta,
        world: UnsafeWorldCell,
    ) -> std::result::Result<(), bevy_ecs::system::SystemParamValidationError> {
        let (_, resource_state, _) = state;
        unsafe {
            <Res<'w, Targets> as bevy_ecs::system::SystemParam>::validate_param(
                resource_state,
                system_meta,
                world,
            )
            .map_err(|err| {
                bevy_ecs::system::SystemParamValidationError::new::<Self>(
                    err.skipped,
                    err.message,
                    "::targets",
                )
            })?;
        }
        Result::Ok(())
    }

    #[inline]
    unsafe fn get_param<'w, 's>(
        state: &'s mut Self::State,
        system_meta: &bevy_ecs::system::SystemMeta,
        world: bevy_ecs::world::unsafe_world_cell::UnsafeWorldCell<'w>,
        change_tick: bevy_ecs::change_detection::Tick,
    ) -> Self::Item<'w, 's> {
        let query = unsafe { Query::get_param(&mut state.0, system_meta, world, change_tick) };
        let targets = unsafe { Res::get_param(&mut state.1, system_meta, world, change_tick) };
        let components = state.2.as_slice();

        SQuery {
            query,
            targets,
            components,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SignalQueryError {
    NoSuchTarget,
    Entity(QueryEntityError),
}

impl<'s, D, F> SQuery<'_, 's, D, F>
where
    D: QueryData,
    F: QueryFilter,
{
    #[inline]
    pub fn get(
        &self,
        target: impl Into<EntityTarget>,
    ) -> Result<ROQueryItem<'_, 's, D>, SignalQueryError> {
        let entity = target.into();

        if let Some(observer) = super::reactive_observer::SignalObserver::get() {
            observer.add_components(entity, self.components);
        }

        let entity = entity
            .get(&self.targets)
            .ok_or(SignalQueryError::NoSuchTarget)?;

        self.query.get(entity).map_err(SignalQueryError::Entity)
    }

    #[inline]
    pub fn iter_many<EntityList: IntoIterator<Item: Into<EntityTarget>>>(
        &self,
        entities: EntityList,
    ) -> impl Iterator<Item = ROQueryItem<'_, 's, D>> {
        entities.into_iter().flat_map(|e| self.get(e).ok())
    }
}

// impl<'s, D, F> SQuery<'_, 's, D, F>
// where
//     D: QueryData,
//     for<'a> ROQueryItem<'a, 's, D>: EntityData,
//     F: QueryFilter,
// {
//     #[inline]
//     pub fn iter(&self) -> impl Iterator<Item = ROQueryItem<'_, 's, D>> {
//         self.query.iter().inspect(|item| {
//             let entity = item.entity();
//             let result = self.query.get(entity).map_err(SignalQueryError::Entity);
//             if result.is_ok()
//                 && let Some(observer) = super::reactive_observer::SignalObserver::get()
//             {
//                 observer.add_components(entity, self.components);
//             }
//         })
//     }

//     #[inline]
//     pub fn single(&self) -> Option<ROQueryItem<'_, 's, D>> {
//         self.query.single().ok().inspect(|item| {
//             let entity = item.entity();
//             let result = self.query.get(entity).map_err(SignalQueryError::Entity);
//             if result.is_ok()
//                 && let Some(observer) = super::reactive_observer::SignalObserver::get()
//             {
//                 observer.add_components(entity, self.components);
//             }
//         })
//     }
// }

// pub trait EntityData {
//     fn entity(&self) -> Entity;
// }

// impl EntityData for Entity {
//     fn entity(&self) -> Entity {
//         *self
//     }
// }

// macro_rules! entity_data {
//     ($($ty:ident),*) => {
//         impl<$($ty),*> EntityData for (Entity, $($ty,)*) {
//             fn entity(&self) -> Entity {
//                 self.0
//             }
//         }
//     };
// }

// variadics_please::all_tuples!(entity_data, 0, 15, T);

impl core::error::Error for SignalQueryError {}

impl core::fmt::Display for SignalQueryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Entity(e) => e.fmt(f),
            Self::NoSuchTarget => {
                write!(f, "Failed to fetch target")
                // write!(f, "The target {t} was not found")
            }
        }
    }
}

// SAFETY: QueryState is constrained to read-only fetches, so it only reads World.
unsafe impl<'w, 's, D: ReadOnlyQueryData + 'static, F: QueryFilter + 'static> ReadOnlySystemParam
    for SQuery<'w, 's, D, F>
{
}
