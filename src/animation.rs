use crate::{js_err::JsErr, runner::app_scope};
use bevy_app::prelude::*;
use bevy_ecs::{prelude::*, schedule::ScheduleLabel};
use js_sys::Function;
use send_wrapper::SendWrapper;

#[derive(Default)]
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct AnimationsPlugin;

impl Plugin for AnimationsPlugin {
    fn build(&self, app: &mut App) {
        app.init_schedule(PreAnimationSchedule)
            .init_schedule(AnimationSchedule)
            .init_resource::<AnimationTime>()
            .init_resource::<AnimationDelta>()
            .init_resource::<AnimationsHandle>()
            .add_systems(Last, start_and_stop_animation_callback)
            .add_systems(PreAnimationSchedule, rate_limit);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ScheduleLabel)]
pub struct PreAnimationSchedule;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ScheduleLabel)]
pub struct AnimationSchedule;

#[derive(Debug, Clone, Default, Resource)]
pub struct AnimationsHandle(Option<(i32, SendWrapper<Function>)>);

#[derive(Default, Clone, Copy, Component)]
pub struct Animation;

#[derive(Default, Clone, Copy, Component)]
pub struct RateLimit {
    rate: f64,
    acc: f64,
    should_update: bool,
}

impl RateLimit {
    pub fn new(rate: f64) -> Self {
        Self {
            rate,
            acc: 0.0,
            should_update: true,
        }
    }

    pub fn should_update(&self) -> bool {
        self.should_update
    }
}

fn rate_limit(delta: Res<AnimationDelta>, mut rate_limit: Query<&mut RateLimit>) {
    for mut rate_limit in rate_limit.iter_mut() {
        rate_limit.acc += delta.0;
        rate_limit.should_update = if rate_limit.acc >= rate_limit.rate {
            rate_limit.acc -= rate_limit.rate;
            true
        } else {
            false
        };
    }
}

#[derive(Default, Clone, Copy, Resource)]
pub struct AnimationDelta(pub f64);

#[derive(Default, Clone, Copy, Resource)]
pub struct AnimationTime(pub f64);

fn start_and_stop_animation_callback(
    mut handle: ResMut<AnimationsHandle>,
    animations: Query<(), With<Animation>>,
    mut animation_time: ResMut<AnimationTime>,
) -> Result {
    use js_sys::Function;
    use wasm_bindgen::{JsCast, closure::Closure};

    let window = web_sys::window().ok_or("browser window should be available")?;

    if animations.is_empty()
        && let Some((id, callback)) = handle.0.take()
    {
        _ = window.cancel_animation_frame(id);
        drop(callback.take());
    }

    if !animations.is_empty() && handle.0.is_none() {
        animation_time.0 = window.performance().unwrap().now();
        let callback = Closure::<dyn Fn()>::new({
            let window = window.clone();
            move || {
                _ = app_scope(|app: &mut App| {
                    let world = app.world_mut();
                    let now = window.performance().unwrap().now();
                    world.resource_scope::<AnimationTime, _>(|world, mut time| {
                        world.insert_resource(AnimationDelta(now - time.0));
                        time.0 = now;
                    });
                    world.run_schedule(PreAnimationSchedule);
                    world.run_schedule(AnimationSchedule);
                    let mut handle = world.resource_mut::<AnimationsHandle>();
                    let (_, callback) = handle.0.take().unwrap();
                    let id = window.request_animation_frame(&callback).unwrap();
                    handle.0 = Some((id, callback));
                });
            }
        });
        let callback = callback.into_js_value().unchecked_into::<Function>();
        let id = window.request_animation_frame(&callback).js_err()?;
        handle.0 = Some((id, SendWrapper::new(callback)));
    }

    Ok(())
}
