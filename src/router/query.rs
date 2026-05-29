use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bevy_platform::collections::HashMap;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::UrlSearchParams;

use crate::{dom::DomSystems, js_err::JsErr, prelude::Window};

#[derive(Default)]
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct QueryPlugin;

impl Plugin for QueryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            QueryParams::update_browser
                .after(DomSystems::ResolveRoutes)
                .run_if(resource_changed::<QueryParams>),
        );
    }
}

#[derive(Resource, Default)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
#[cfg_attr(feature = "reflect", reflect(Resource))]
pub struct QueryParams(HashMap<String, String>);

impl core::ops::Deref for QueryParams {
    type Target = HashMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl core::ops::DerefMut for QueryParams {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl QueryParams {
    pub(crate) fn from_url(url: &web_sys::Url) -> Self {
        let mut params = Self::default();
        params.update(url);
        params
    }

    pub(crate) fn update(&mut self, url: &web_sys::Url) {
        self.clear();

        let js_params = url.search_params();
        for pair in js_params.entries() {
            let pair: js_sys::Array = pair.unwrap().unchecked_into();
            let key = pair.get(0).as_string().unwrap();
            let value = pair.get(1).as_string().unwrap();
            self.insert(key, value);
        }
    }

    fn update_browser(window: Single<&Window>, params: Res<QueryParams>) -> Result {
        let location = window.location();
        let url = web_sys::Url::new(&location.href().js_err()?).js_err()?;

        let js_params = UrlSearchParams::new().js_err()?;
        for (key, value) in params.iter() {
            js_params.set(key, value);
        }

        url.set_search(&String::from(js_params.to_string()));

        window
            .history()
            .unwrap()
            .replace_state_with_url(&JsValue::NULL, "", Some(&String::from(url.to_string())))
            .unwrap();

        Ok(())
    }
}
