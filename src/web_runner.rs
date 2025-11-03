use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use futures::{StreamExt, channel::mpsc};
use std::cell::RefCell;

pub struct WebRunnerPlugin;

#[derive(Resource)]
pub struct ScheduleTrigger(mpsc::UnboundedSender<()>);

impl ScheduleTrigger {
    pub fn trigger(&self) {
        self.0
            .unbounded_send(())
            .expect("failed to send schedule trigger");
    }
}

thread_local! {
    static APP: RefCell<App> = panic!("world not initialized");
}

pub fn app_scope<F, R>(func: F) -> R
where
    F: FnOnce(&mut App) -> R,
{
    APP.with(|app| {
        let mut app = app.borrow_mut();

        func(&mut app)
    })
}

impl Plugin for WebRunnerPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = mpsc::unbounded();

        app.insert_resource(ScheduleTrigger(tx))
            .set_runner(web_runner(rx));
    }
}

fn web_runner(mut receiver: mpsc::UnboundedReceiver<()>) -> impl FnOnce(App) -> AppExit + 'static {
    move |app: App| {
        app.world().resource::<ScheduleTrigger>().trigger();
        APP.set(app);

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                if receiver.next().await.is_some() {
                    while receiver.try_next().is_ok_and(|r| r.is_some()) {}

                    let exit = app_scope(|app| {
                        match app.plugins_state() {
                            bevy_app::PluginsState::Adding => {
                                // ?
                                app.world().resource::<ScheduleTrigger>().trigger();
                            }
                            bevy_app::PluginsState::Ready => {
                                app.finish();
                                app.world().resource::<ScheduleTrigger>().trigger();
                            }
                            bevy_app::PluginsState::Finished => {
                                app.cleanup();
                                app.world().resource::<ScheduleTrigger>().trigger();
                            }
                            bevy_app::PluginsState::Cleaned => {
                                app.update();
                            }
                        }

                        app.should_exit()
                    });

                    if let Some(exit) = exit {
                        log::info!("App exiting: {exit:?}");
                        break;
                    }
                }
            }
        });

        AppExit::Success
    }
}
