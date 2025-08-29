#![allow(clippy::type_complexity)]

use bevy_app::prelude::*;

pub mod dom;
pub mod js_err;
mod web_runner;

#[cfg(feature = "router")]
pub mod router;

pub struct BwebPlugin;

impl Plugin for BwebPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            web_runner::WebRunnerPlugin,
            dom::DomPlugin,
            #[cfg(feature = "router")]
            router::RouterPlugin,
        ));
    }
}
