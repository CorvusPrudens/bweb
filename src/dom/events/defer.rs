use crate::js_err::JsErr;

pub trait DeferredEvents {
    fn focus_deferred(&self);
    fn blur_deferred(&self);
}

impl DeferredEvents for web_sys::HtmlElement {
    fn blur_deferred(&self) {
        let el = self.clone();
        crate::task::spawn_local(async move |_| el.blur().js_err());
    }

    fn focus_deferred(&self) {
        let el = self.clone();
        crate::task::spawn_local(async move |_| el.focus().js_err());
    }
}
