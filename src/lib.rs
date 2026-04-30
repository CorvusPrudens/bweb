//! Interact with the DOM in the vocabulary of ECS.
//!
//! `bweb` provides components, events, and a runner that
//! translate browser APIs into idiomatic ECS APIs.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(clippy::type_complexity)]

pub mod dom;
pub mod js_err;
pub mod relative_mouse;
pub mod runner;
pub mod task;
pub mod time;

#[cfg(feature = "router")]
pub mod router;

pub mod prelude {
    pub use crate::dom::prelude::*;
    pub use crate::js_err::JsErr;
    pub use crate::task::{TaskComponent, TaskWorld, spawn_local};
    pub use crate::time::sleep;

    #[cfg(feature = "router")]
    pub use crate::router::Route;

    pub use crate::BwebPlugins;
}

bevy_app::plugin_group! {
    /// `bweb`'s top-level plugin.
    #[cfg_attr(feature = "debug", derive(Debug))]
    pub struct BwebPlugins {
        runner:::WebRunnerPlugin,
        dom:::DomPlugin,
        relative_mouse:::RelativeMousePlugin,
        #[cfg(feature = "router")]
        router:::RouterPlugin,
    }
}
