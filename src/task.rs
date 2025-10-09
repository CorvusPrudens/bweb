#![allow(async_fn_in_trait)]

use bevy_ecs::{
    error::ErrorContext,
    prelude::*,
    system::{RunSystemError, RunSystemOnce},
};

use crate::web_runner::ScheduleTrigger;

/// A handle providing access to the world within tasks.
///
/// This is particularly useful in async contexts when
/// constant access to the world isn't possible.
///
/// While this struct is just a ZST, it prevents
/// misuse of the underlying world.
pub struct TaskWorld(());

impl TaskWorld {
    /// Run a one-shot system.
    pub fn run<S, O, M>(&mut self, system: S) -> Result<O>
    where
        S: IntoSystem<(), O, M>,
    {
        self.with(|world| world.run_system_once(system))
            .map_err(|e| match e {
                RunSystemError::Failed(f) => f,
                RunSystemError::Skipped(s) => s.into(),
            })
    }

    /// Run a closure with access to the world.
    ///
    /// This will unconditionally trigger a schedule run.
    pub fn with<F, O>(&mut self, func: F) -> O
    where
        F: FnOnce(&mut World) -> O,
    {
        crate::web_runner::app_scope(|app| {
            let world = app.world_mut();
            world.resource::<ScheduleTrigger>().trigger();
            let result = func(world);
            world.flush();
            result
        })
    }
}

pub trait AsyncTask<M>: 'static {
    async fn run(self, world: TaskWorld) -> Result;
}

pub struct InfallibleTask;
pub struct FallibleTask;

impl<F> AsyncTask<InfallibleTask> for F
where
    F: AsyncFnOnce(TaskWorld) + 'static,
{
    async fn run(self, world: TaskWorld) -> Result {
        Ok(self(world).await)
    }
}

impl<F> AsyncTask<FallibleTask> for F
where
    F: AsyncFnOnce(TaskWorld) -> Result + 'static,
{
    async fn run(self, world: TaskWorld) -> Result {
        self(world).await
    }
}

/// Spawn an async task.
pub fn spawn_local<F, M>(task: F)
where
    F: AsyncTask<M>,
{
    wasm_bindgen_futures::spawn_local(async move {
        let world = TaskWorld(());
        let result = task.run(world).await;

        crate::web_runner::app_scope(|app| match result {
            Err(e) => {
                let tick = app.world_mut().change_tick();
                match app.get_error_handler() {
                    Some(error_handler) => error_handler(
                        e,
                        ErrorContext::System {
                            name: bevy_utils::prelude::DebugName::type_name::<F>(),
                            last_run: tick,
                        },
                    ),
                    None => {
                        log::error!("Failed to execute async task: {e:?}");
                    }
                }
            }
            Ok(()) => {}
        })
    })
}
