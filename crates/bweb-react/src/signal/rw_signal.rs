use std::sync::Arc;

use super::{
    SignalInner, SignalReadGuard, SignalWriteGuard,
    traits::{Read, Write},
};

// TODO: technically this is twice as much stack as we need, but
// this is rarely used so...
pub struct RwSignal<T> {
    read: ReadSignal<T>,
    write: WriteSignal<T>,
}

impl<T> RwSignal<T> {
    pub fn new(value: T) -> Self {
        let inner = Arc::new(SignalInner::new(Some(value)));

        Self {
            read: ReadSignal {
                data: Arc::clone(&inner),
            },
            write: WriteSignal { data: inner },
        }
    }

    pub fn split(self) -> (ReadSignal<T>, WriteSignal<T>) {
        let RwSignal { read, write } = self;
        (read, write)
    }
}

impl<T> Clone for RwSignal<T> {
    fn clone(&self) -> Self {
        Self {
            read: self.read.clone(),
            write: self.write.clone(),
        }
    }
}

impl<T: Send + Sync + 'static> Read for RwSignal<T> {
    type Value = T;
    type Guard<'a>
        = SignalReadGuard<'a, T>
    where
        Self: 'a;

    fn try_read(&self) -> Option<Self::Guard<'_>> {
        self.read.try_read()
    }
}

impl<T> Write for RwSignal<T> {
    type Value = T;
    type Guard<'a>
        = SignalWriteGuard<'a, T>
    where
        Self: 'a;

    fn try_write(&self) -> Option<Self::Guard<'_>> {
        self.write.try_write()
    }
}

pub fn signal<T>(value: T) -> (ReadSignal<T>, WriteSignal<T>) {
    RwSignal::new(value).split()
}

pub struct ReadSignal<T> {
    data: Arc<SignalInner<T>>,
}

impl<T> Clone for ReadSignal<T> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
        }
    }
}

pub struct WriteSignal<T> {
    data: Arc<SignalInner<T>>,
}

impl<T> Clone for WriteSignal<T> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
        }
    }
}

impl<T: Send + Sync + 'static> Read for ReadSignal<T> {
    type Value = T;
    type Guard<'a>
        = SignalReadGuard<'a, T>
    where
        Self: 'a;

    fn try_read(&self) -> Option<Self::Guard<'_>> {
        if let Some(observer) = super::reactive_observer::SignalObserver::get() {
            observer.add_signal(super::reactive_observer::SignalSubscriber::new(&self.data));
        }

        self.data.try_read()
    }
}

impl<T> Write for WriteSignal<T> {
    type Value = T;
    type Guard<'a>
        = SignalWriteGuard<'a, T>
    where
        Self: 'a;

    fn try_write(&self) -> Option<Self::Guard<'_>> {
        self.data.try_write()
    }
}
