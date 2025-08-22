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
                    let exit = APP.with(|app| {
                        let mut app = app.borrow_mut();

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
                        bevy_log::info!("App exiting: {exit:?}");
                        break;
                    }
                }
            }
        });

        // let func = Rc::<RefCell<Option<Closure<dyn FnMut()>>>>::new(RefCell::new(None));
        // let function = wasm_bindgen::closure::Closure::new({
        //     let func = func.clone();
        //     let window = window.clone();
        //     move || {
        //         match app.plugins_state() {
        //             bevy_app::PluginsState::Adding => {
        //                 // ?
        //             }
        //             bevy_app::PluginsState::Ready => {
        //                 app.finish();
        //             }
        //             bevy_app::PluginsState::Finished => {
        //                 app.cleanup();
        //             }
        //             bevy_app::PluginsState::Cleaned => {
        //                 app.update();
        //             }
        //         }
        //
        //         if let Some(exit) = app.should_exit() {
        //             bevy_log::info!("App exited: {exit:?}");
        //         } else {
        //             window
        //                 .request_animation_frame(
        //                     func.borrow().as_ref().unwrap().as_ref().unchecked_ref(),
        //                 )
        //                 .unwrap();
        //         }
        //     }
        // });
        // *func.borrow_mut() = Some(function);
        // window
        //     .request_animation_frame(func.borrow().as_ref().unwrap().as_ref().unchecked_ref())
        //     .unwrap();

        AppExit::Success
    }
}
