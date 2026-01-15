use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use futures::{Stream, StreamExt, task::AtomicWaker};
use std::{
    cell::{BorrowMutError, RefCell},
    sync::{Arc, atomic::AtomicBool},
};

pub struct WebRunnerPlugin;

#[derive(Resource)]
pub struct ScheduleTrigger {
    task_pool: Notifier,
    microtask_trigger: bool,
}

struct Inner {
    waker: AtomicWaker,
    set: AtomicBool,
}

struct Notifier(Arc<Inner>);
struct Observer(Arc<Inner>);

fn notifier() -> (Notifier, Observer) {
    let inner = Arc::new(Inner {
        waker: AtomicWaker::new(),
        set: AtomicBool::new(false),
    });
    let rx = Arc::clone(&inner);

    (Notifier(inner), Observer(rx))
}

impl Notifier {
    pub fn notify(&self) {
        self.0.set.store(true, std::sync::atomic::Ordering::Relaxed);
        self.0.waker.wake();
    }
}

impl Stream for Observer {
    type Item = ();

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.0.waker.register(cx.waker());

        if self.0.set.swap(false, std::sync::atomic::Ordering::Relaxed) {
            std::task::Poll::Ready(Some(()))
        } else {
            std::task::Poll::Pending
        }
    }
}

impl ScheduleTrigger {
    pub fn trigger(&mut self) {
        if !self.microtask_trigger {
            self.microtask_trigger = true;
            crate::task::app_scope_microtask(|app| {
                app.world_mut().resource_mut::<Self>().microtask_trigger = false;
                app.update();
            });
        }
    }

    pub fn trigger_async(&self) {
        self.task_pool.notify();
    }
}

thread_local! {
    static APP: RefCell<App> = panic!("world not initialized");
}

pub fn app_scope<F, R>(func: F) -> Result<R, BorrowMutError>
where
    F: FnOnce(&mut App) -> R,
{
    APP.with(|app| {
        let mut app = app.try_borrow_mut()?;
        Ok(func(&mut app))
    })
}

impl Plugin for WebRunnerPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = notifier();

        app.insert_resource(ScheduleTrigger {
            microtask_trigger: false,
            task_pool: tx,
        })
        .set_runner(web_runner(rx));
    }
}

fn web_runner(mut receiver: Observer) -> impl FnOnce(App) -> AppExit + 'static {
    move |app: App| {
        app.world().resource::<ScheduleTrigger>().trigger_async();
        APP.set(app);

        wasm_bindgen_futures::spawn_local(async move {
            while receiver.next().await.is_some() {
                let exit = app_scope(|app| {
                    match app.plugins_state() {
                        bevy_app::PluginsState::Adding => {
                            // ?
                            app.world().resource::<ScheduleTrigger>().trigger_async();
                        }
                        bevy_app::PluginsState::Ready => {
                            app.finish();
                            app.world().resource::<ScheduleTrigger>().trigger_async();
                        }
                        bevy_app::PluginsState::Finished => {
                            app.cleanup();
                            app.world().resource::<ScheduleTrigger>().trigger_async();
                        }
                        bevy_app::PluginsState::Cleaned => {
                            app.update();
                        }
                    }

                    app.should_exit()
                });

                if let Ok(Some(exit)) = exit {
                    log::info!("App exiting: {exit:?}");
                    break;
                }
            }
        });

        AppExit::Success
    }
}
