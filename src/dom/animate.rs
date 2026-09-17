use js_sys::{Array, Object, Reflect};
use wasm_bindgen::JsValue;
use web_sys::{Animation, Element};

/// A small wrapper around keyframe options for the
/// Web Animations API.
pub struct Keyframes(Object);

impl Default for Keyframes {
    fn default() -> Self {
        Self::new()
    }
}

impl Keyframes {
    pub fn new() -> Self {
        Self(Object::new())
    }

    pub fn property<'a>(self, name: &str, values: impl IntoIterator<Item = &'a str>) -> Self {
        let values: Array = values.into_iter().map(JsValue::from_str).collect();
        self.set(name, &values)
    }

    pub fn offsets(self, offsets: impl IntoIterator<Item = f64>) -> Self {
        let offsets: Array = offsets.into_iter().map(JsValue::from_f64).collect();
        self.set("offset", &offsets)
    }

    pub fn easing(self, easing: &str) -> Self {
        self.set("easing", &JsValue::from_str(easing))
    }

    fn set(self, key: &str, value: &JsValue) -> Self {
        let _ = Reflect::set(&self.0, &JsValue::from_str(key), value);
        self
    }
}

pub fn animate(element: &Element, keyframes: &Keyframes, duration_ms: f64) -> Animation {
    element.animate_with_f64(Some(&keyframes.0), duration_ms)
}
