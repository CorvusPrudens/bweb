use core::ops::{Deref, DerefMut};

/// Provides read-only access to a signal's value.
pub trait Read {
    type Value;
    type Guard<'a>: Deref<Target = Self::Value>
    where
        Self: 'a;

    fn try_read(&self) -> Option<Self::Guard<'_>>;

    fn read(&self) -> Self::Guard<'_> {
        self.try_read().expect("attempted to read signal")
    }
}

/// Provides mutable access to a signal's value.
pub trait Write {
    type Value;
    type Guard<'a>: DerefMut<Target = Self::Value>
    where
        Self: 'a;

    fn try_write(&self) -> Option<Self::Guard<'_>>;

    fn write(&self) -> Self::Guard<'_> {
        self.try_write().expect("attempted to write signal")
    }
}

pub trait Get {
    type Value;

    fn try_get(&self) -> Option<Self::Value>;

    fn get(&self) -> Self::Value {
        self.try_get().expect("attempted to get signal")
    }
}

impl<T: Read> Get for T
where
    T::Value: Clone,
{
    type Value = T::Value;

    fn try_get(&self) -> Option<Self::Value> {
        self.try_read().map(|v| v.clone())
    }
}

pub trait Set {
    type Value;

    fn try_set(&self, value: Self::Value) -> Option<Self::Value>;

    fn set(&self, value: Self::Value) {
        if self.try_set(value).is_some() {
            panic!("failed to set signal value");
        }
    }
}

impl<T: Write> Set for T {
    type Value = T::Value;

    fn try_set(&self, value: Self::Value) -> Option<Self::Value> {
        match self.try_write() {
            Some(mut guard) => {
                *guard = value;
                None
            }
            None => Some(value),
        }
    }
}
