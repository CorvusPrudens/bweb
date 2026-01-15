#![allow(async_fn_in_trait, clippy::unit_arg, clippy::single_match)]

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
    pub fn resource<R: Resource + Clone>(&mut self) -> R {
        self.with_trigger(false, |world| world.resource::<R>().clone())
    }

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
        Self::with_trigger(self, true, func)
    }

    /// Run a closure with access to the world.
    ///
    /// If `trigger` is `true`, the main schedule will execute
    /// at the next await point.
    pub fn with_trigger<F, O>(&mut self, trigger: bool, func: F) -> O
    where
        F: FnOnce(&mut World) -> O,
    {
        crate::web_runner::app_scope(|app| {
            let world = app.world_mut();
            if trigger {
                world.resource::<ScheduleTrigger>().trigger_async();
            }
            let result = func(world);
            world.flush();
            result
        })
        .expect("failed to borrow app")
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

        let res = crate::web_runner::app_scope(|app| match result {
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
        });

        if res.is_err() {
            log::error!("Failed to borrow app in task.");
        }
    })
}

/// A task that cancels when dropped.
#[derive(Component, Debug)]
pub struct TaskComponent(Option<futures::channel::oneshot::Sender<()>>);

impl TaskComponent {
    pub fn new<T, M>(task: T) -> Self
    where
        T: AsyncTask<M>,
    {
        let (tx, mut rx) = futures::channel::oneshot::channel();
        spawn_local(async move |world: TaskWorld| {
            use futures::FutureExt;

            futures::select! {
                result = task.run(world).fuse() => result,
                _ = rx => Ok(())
            }
        });

        Self(Some(tx))
    }
}

impl Drop for TaskComponent {
    fn drop(&mut self) {
        let _ = self.0.take().unwrap().send(());
    }
}

pub trait Microtask<M>: 'static {
    fn run(self, world: &mut World) -> Result;
}

impl<F> Microtask<InfallibleTask> for F
where
    F: FnOnce(&mut World) + 'static,
{
    fn run(self, world: &mut World) -> Result {
        Ok(self(world))
    }
}

impl<F> Microtask<FallibleTask> for F
where
    F: FnOnce(&mut World) -> Result + 'static,
{
    fn run(self, world: &mut World) -> Result {
        self(world)
    }
}

/// A microtask is a short function which will run after the current task has
/// completed its work and when there is no other code waiting to be run before
/// control of the execution context is returned to the browser's event loop.
///
/// Microtasks are especially useful for libraries and frameworks that need
/// to perform final cleanup or other just-before-rendering tasks.
///
/// [MDN queueMicrotask](https://developer.mozilla.org/en-US/docs/Web/API/queueMicrotask)
pub fn queue_microtask<F, M>(task: F)
where
    F: Microtask<M>,
{
    app_scope_microtask(move |app| {
        let result = task.run(app.world_mut());

        match result {
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
        }
    });
}

pub(crate) fn app_scope_microtask<F>(task: F)
where
    F: FnOnce(&mut bevy_app::App) + 'static,
{
    use js_sys::{Function, Reflect};
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};

    let window = web_sys::window().expect("Attempted to queue microtask on non-web platform");

    let task = Closure::once_into_js(move || {
        let res = crate::web_runner::app_scope(task);
        if res.is_err() {
            log::error!("Failed to borrow app for microtask.");
        }
    });

    let queue_microtask = Reflect::get(&window, &JsValue::from_str("queueMicrotask"))
        .expect("queueMicrotask not available");
    let queue_microtask = queue_microtask.unchecked_into::<Function>();
    _ = queue_microtask.call1(&JsValue::UNDEFINED, &task);
}
