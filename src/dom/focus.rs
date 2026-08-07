//! Focus primitives for manual management.

use crate::js_err::JsErr;
use bevy_ecs::error::Result;
use wasm_bindgen::JsCast;

/// What the browser treats as a tab stop.
const FOCUSABLE: &str = ":is(button, [href], input, select, textarea, [tabindex])\
                         :not([tabindex='-1']):not(:disabled)";

pub trait FocusableDescendants {
    /// This element's focusable descendants in DOM order.
    fn focusable_descendants(&self) -> Result<Vec<web_sys::HtmlElement>>;
}

impl FocusableDescendants for web_sys::Element {
    fn focusable_descendants(&self) -> Result<Vec<web_sys::HtmlElement>> {
        let list = self.query_selector_all(FOCUSABLE).js_err()?;

        Ok((0..list.length())
            .filter_map(|i| list.get(i))
            .filter_map(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
            .collect())
    }
}

pub trait FocusNoScroll {
    /// Focus without letting the browser scroll an ancestor to reveal this
    /// element.
    fn focus_no_scroll(&self) -> Result<()>;

    /// A deferred [`focus_no_scroll`](Self::focus_no_scroll).
    ///
    /// Focusing dispatches `blur` and `focus` synchronously, causing
    /// reentrancy problems when called in systems.
    fn focus_no_scroll_deferred(&self);
}

impl FocusNoScroll for web_sys::HtmlElement {
    fn focus_no_scroll(&self) -> Result<()> {
        let options = web_sys::FocusOptions::new();
        options.set_prevent_scroll(true);
        self.focus_with_options(&options).js_err()
    }

    fn focus_no_scroll_deferred(&self) {
        let element = self.clone();
        crate::task::spawn_local(async move |_| element.focus_no_scroll());
    }
}
