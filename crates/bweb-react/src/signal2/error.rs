use std::ops::Deref;
use std::sync::RwLockReadGuard;

/// Error returned by fallible signal reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalError {
    /// The signal has no value yet — it hasn't evaluated, or its last evaluation
    /// produced `Err` (e.g. an upstream source was itself not ready).
    NotReady,
}

impl core::fmt::Display for SignalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotReady => write!(f, "signal has no value yet"),
        }
    }
}

impl core::error::Error for SignalError {}

pub type SignalResult<O> = Result<O, SignalError>;

/// A read guard over a signal's value.
///
/// Holds the underlying `RwLock` read guard and dereferences straight to the
/// value. Only constructed when the value is `Some`, and the read lock is held
/// for the guard's lifetime, so the `unwrap` in `deref` cannot fail.
pub struct SignalReadGuard<'a, O>(pub(super) RwLockReadGuard<'a, Option<O>>);

impl<O> Deref for SignalReadGuard<'_, O> {
    type Target = O;

    fn deref(&self) -> &O {
        self.0.as_ref().expect("SignalReadGuard over a None value")
    }
}
