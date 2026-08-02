//! A plain value parked in a component, so a derived signal can produce one.
//!
//! Everything in this graph is arranged around components: a signal's value is
//! a component on the entity it watches, and [`derive`] writes its output onto
//! the node's own entity. That is what makes a derived signal indistinguishable
//! from a source signal to everything downstream — but it also means a derived
//! signal cannot yield a `bool`, an `Option<String>`, or a `Vec<Entity>`, none
//! of which are components.
//!
//! [`Derived`] is the newtype that closes that gap once instead of at every
//! call site. [`derive_value`] wraps for you and [`SourceSignal::value`] reads
//! back through it, so the wrapper is mostly invisible; what it costs is one
//! component type per `T` rather than per site.
//!
//! [`derive`]: super::SignalExt::derive
//! [`derive_value`]: super::SignalExt::derive_value
//! [`SourceSignal::value`]: SourceSignal::value

use core::ops::{Deref, DerefMut};

use bevy_ecs::prelude::*;

use crate::signal2::{
    ReactError,
    dynamic::Signal,
    list::ListSource,
    source::{SignalStore, SourceSignal},
};

/// A value that is not a component, in a component.
///
/// Build one with [`derive_value`](super::SignalExt::derive_value) rather than
/// by hand; read it back with [`SourceSignal::value`] or
/// [`SourceSignal::project`].
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Derived<T: Send + Sync + 'static>(pub T);

impl<T: Send + Sync + 'static> Deref for Derived<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Send + Sync + 'static> DerefMut for Derived<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Send + Sync + 'static> From<T> for Derived<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

/// A wrapped `Vec` drives a list the same way a relationship collection does.
///
/// This is what lets [`derive_value`] feed a [`ReactiveList`] — the derived
/// signal produces the collection, and the list reads its elements straight out
/// of the component without ever cloning the whole thing.
///
/// [`derive_value`]: super::SignalExt::derive_value
/// [`ReactiveList`]: super::list::ReactiveList
impl<T: Send + Sync + 'static> ListSource for Derived<Vec<T>> {
    type Item = T;

    fn items(&self) -> &[T] {
        &self.0
    }
}

impl<T: Send + Sync + 'static> SourceSignal<&'static Derived<T>> {
    /// Read the wrapped value, subscribing the node currently being evaluated.
    ///
    /// [`get`](SourceSignal::get) with the newtype peeled off, which is what
    /// every reader of a `derive_value` signal actually wants.
    pub fn value<'a>(&self, store: &'a SignalStore<'_, '_>) -> Result<&'a T, ReactError> {
        self.get(store).map(|derived| &derived.0)
    }

    /// This signal as a [`Signal<T>`], with the newtype peeled off.
    ///
    /// For handing a derived value to something that takes a `Signal<T>` —
    /// [`watch_signal`] most of all, which is how a `Signal<Entity>` derived
    /// from anything at all becomes a binding.
    ///
    /// [`watch_signal`]: super::source::SourceSignalView::watch_signal
    pub fn project(&self) -> Signal<T> {
        Signal::project(self.clone(), |derived| &derived.0)
    }
}
