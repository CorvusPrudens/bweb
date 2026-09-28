//! Timers on the JavaScript event loop.

use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
    time::Duration,
};
use wasm_bindgen::{closure::Closure, prelude::*};

#[wasm_bindgen]
extern "C" {
    // globals mean this should work in most contexts
    #[wasm_bindgen(js_name = setTimeout)]
    fn set_timeout(handler: &Closure<dyn FnMut()>, ms: i32) -> i32;

    #[wasm_bindgen(js_name = clearTimeout)]
    fn clear_timeout(id: i32);
}

#[derive(Default)]
struct State {
    fired: bool,
    waker: Option<Waker>,
}

/// Resolves once its duration has passed. Dropping it cancels the
/// timer.
#[must_use = "`Sleep` does nothing unless awaited"]
pub struct Sleep {
    id: i32,
    state: Rc<RefCell<State>>,
    _callback: Closure<dyn FnMut()>,
}

/// Wait for `duration`. A zero duration still yields to the event loop.
pub fn sleep(duration: Duration) -> Sleep {
    let state = Rc::new(RefCell::new(State::default()));
    let callback = Closure::new({
        let state = state.clone();
        move || {
            let waker = {
                let mut state = state.borrow_mut();
                state.fired = true;
                state.waker.take()
            };
            // Outside the borrow, in case waking polls right away.
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    });
    let ms = duration.as_millis().min(i32::MAX as u128) as i32;
    let id = set_timeout(&callback, ms);

    Sleep {
        id,
        state,
        _callback: callback,
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.state.borrow_mut();
        if state.fired {
            Poll::Ready(())
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if !self.state.borrow().fired {
            clear_timeout(self.id);
        }
    }
}
