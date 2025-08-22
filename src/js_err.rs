use bevy_ecs::error::Result;
use wasm_bindgen::JsValue;

/// Convert a `Result<T, JsValue>` into a `bevy::Result`.
pub trait JsErr<T> {
    fn js_err(self) -> Result<T>;
}

impl<T> JsErr<T> for core::result::Result<T, JsValue> {
    fn js_err(self) -> Result<T> {
        match self {
            Ok(value) => Ok(value),
            Err(e) => {
                let error = format!("{:?}", e);
                Err(error.into())
            }
        }
    }
}
