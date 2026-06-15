use crate::dom::events::EventOf;
use crate::dom::html::HtmlElement;
use crate::js_err::JsErr;
use bevy_ecs::prelude::*;
use bevy_ecs::system::{Query, SystemParam};
use wasm_bindgen::JsValue;

#[derive(SystemParam, Debug)]
pub struct InputValue<'w, 's> {
    ev: Query<'w, 's, &'static EventOf>,
    q: Query<'w, 's, &'static HtmlElement>,
}

impl InputValue<'_, '_> {
    pub fn get(&self, entity: Entity) -> Result<String> {
        thread_local! {
            static VALUE: JsValue = "value".into();
        }

        let el = self.ev.get(entity)?;
        let node = self.q.get(el.0)?;

        let value = VALUE
            .with(|value| js_sys::Reflect::get(node, value))
            .js_err()?
            .as_string()
            .ok_or("expected string from input value")?;

        Ok(value)
    }
}
