use bevy_ecs::{component::ComponentId, prelude::*};

pub struct SRes<'w, R: Resource> {
    res: Res<'w, R>,
    id: ComponentId,
}

type ResAlias<'w, 's, R> = (Res<'w, R>,);

#[doc(hidden)]
pub struct FetchState<R: Resource> {
    state: <ResAlias<'static, 'static, R> as bevy_ecs::system::SystemParam>::State,
}

unsafe impl<R: Resource> bevy_ecs::system::SystemParam for SRes<'_, R> {
    type State = FetchState<R>;
    type Item<'w, 's> = SRes<'w, R>;
    fn init_state(world: &mut bevy_ecs::world::World) -> Self::State {
        FetchState {
            state: <ResAlias<'_, '_, R> as bevy_ecs::system::SystemParam>::init_state(world),
        }
    }

    fn init_access(
        state: &Self::State,
        system_meta: &mut bevy_ecs::system::SystemMeta,
        component_access_set: &mut bevy_ecs::query::FilteredAccessSet,
        world: &mut bevy_ecs::world::World,
    ) {
        <ResAlias<'_, '_, R> as bevy_ecs::system::SystemParam>::init_access(
            &state.state,
            system_meta,
            component_access_set,
            world,
        );
    }

    fn apply(
        state: &mut Self::State,
        system_meta: &bevy_ecs::system::SystemMeta,
        world: &mut bevy_ecs::world::World,
    ) {
        <ResAlias<'_, '_, R> as bevy_ecs::system::SystemParam>::apply(
            &mut state.state,
            system_meta,
            world,
        );
    }

    fn queue(
        state: &mut Self::State,
        system_meta: &bevy_ecs::system::SystemMeta,
        world: bevy_ecs::world::DeferredWorld,
    ) {
        <ResAlias<'_, '_, R> as bevy_ecs::system::SystemParam>::queue(
            &mut state.state,
            system_meta,
            world,
        );
    }

    #[inline]
    unsafe fn validate_param<'w, 's>(
        state: &'s mut Self::State,
        _system_meta: &bevy_ecs::system::SystemMeta,
        _world: bevy_ecs::world::unsafe_world_cell::UnsafeWorldCell<'w>,
    ) -> Result<(), bevy_ecs::system::SystemParamValidationError> {
        let FetchState { state: (f0,) } = state;
        unsafe {
            <Res<'w, R> as bevy_ecs::system::SystemParam>::validate_param(f0, _system_meta, _world)
                .map_err(|err| {
                    bevy_ecs::system::SystemParamValidationError::new::<Self>(
                        err.skipped,
                        err.message,
                        "::res",
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
        let id = state.state.0;

        let (res,) = unsafe {
            <(Res<'w, R>,) as bevy_ecs::system::SystemParam>::get_param(
                &mut state.state,
                system_meta,
                world,
                change_tick,
            )
        };

        SRes { res, id }
    }
}

unsafe impl<'w, 's, R: Resource> bevy_ecs::system::ReadOnlySystemParam for SRes<'w, R> where
    Res<'w, R>: bevy_ecs::system::ReadOnlySystemParam
{
}

impl<R: Resource> core::ops::Deref for SRes<'_, R> {
    type Target = R;

    fn deref(&self) -> &Self::Target {
        if let Some(observer) = super::reactive_observer::SignalObserver::get() {
            observer.add_resource(self.id);
        }

        &self.res
    }
}
