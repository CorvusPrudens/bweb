//! One handle over every way a value can be supplied.
//!
//! A view that takes a label does not care whether the caller has a `&'static
//! str`, a [`Cell`] someone types into, or a derived signal folded out of three
//! others. [`Signal`] is the type it takes instead: a value that may or may not
//! be reactive, read the same way either way, subscribing the reader to whatever
//! is actually behind it.
//!
//! # How the subscription happens
//!
//! Nothing new. The graph's currency for "a node read something" is already
//! erased — [`SignalReads`] holds `(Entity, SubscribeFn)` pairs, and [`Cell`]
//! already proves a source can join that list without the reader knowing what
//! kind of source it is. A [`Signal`] just calls the underlying handle's own
//! `get`, which records the read exactly as a direct call would.
//!
//! So a [`Signal`] costs a branch on the read path and nothing at all on the
//! scan path. A `Value` records no read, which is not a special case to work
//! around but the right answer: a node reading only static signals genuinely has
//! no dependencies and must never be woken.
//!
//! # Why the source variant is a closure
//!
//! It could be a pair of function pointers, at the price of forcing `T:
//! Component` on the whole type — and `Entity` is not a `Component`, so the
//! source variant of `Signal<Entity>` would be uninhabited. That is precisely
//! the case [`watch_signal`] needs. A boxed closure erases the component type
//! instead of the value type, which keeps `Signal<Entity>` and `Signal<bool>`
//! legal and buys [`project`](Signal::project) as well. The cost is one
//! allocation per source-backed signal, paid once at build time next to an
//! entity spawn.
//!
//! [`SignalReads`]: super::source::SignalReads
//! [`watch_signal`]: super::source::SourceSignalView::watch_signal

use core::ops::Deref;
use std::sync::Arc;

use bevy_ecs::prelude::*;

use crate::signal2::{
    ReactError,
    cell::{Cell, CellRef},
    source::{SignalStore, SourceSignal},
};

/// Reads a source-backed signal, with the component type erased.
type ReadFn<T> =
    Arc<dyn for<'a, 'w, 's> Fn(&'a SignalStore<'w, 's>) -> Result<&'a T, ReactError> + Send + Sync>;

/// A value that may or may not be reactive.
///
/// Build one from a plain value, a [`Cell`], or any [`SourceSignal`] — which
/// covers derived signals too, since a derived signal *is* a `SourceSignal` over
/// its output component. Read it with [`get`](Self::get) from inside a derived
/// closure or an effect and the reader is subscribed to whatever is behind it;
/// read a `Value` and nothing is subscribed, because there is nothing to wake.
///
/// Cloning is cheap regardless of `T`: every variant is behind an `Arc`.
pub struct Signal<T>(SignalKind<T>);

enum SignalKind<T> {
    /// Behind an `Arc` so that `Signal<T>: Clone` holds without `T: Clone`, and
    /// so cloning a signal never copies the value.
    Value(Arc<T>),
    Cell(Cell<T>),
    Source(ReadFn<T>),
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self(match &self.0 {
            SignalKind::Value(value) => SignalKind::Value(Arc::clone(value)),
            SignalKind::Cell(cell) => SignalKind::Cell(cell.clone()),
            SignalKind::Source(read) => SignalKind::Source(Arc::clone(read)),
        })
    }
}

impl<T: Send + Sync + 'static> Signal<T> {
    /// A signal that never changes.
    ///
    /// There is deliberately no blanket `From<T>`: it would collide with core's
    /// `impl<T> From<T> for T`, which coherence cannot rule out for a generic
    /// wrapper. Concrete conversions are provided where they are worth the
    /// ergonomics — see [`From<Entity>`](Signal::from).
    pub fn value(value: T) -> Self {
        Self(SignalKind::Value(Arc::new(value)))
    }

    /// Read a `C`-valued signal as some part of it.
    ///
    /// The way to get a source-backed signal of a type that is not itself a
    /// component — `Signal<Entity>` from a component wrapping one, or a field
    /// out of a larger struct — without paying for a derived node.
    ///
    /// ```ignore
    /// #[derive(Component)] struct Selected(Entity);
    /// let selected = commands.signal::<&Selected>().watch(model);
    /// let entity: Signal<Entity> = Signal::project(selected, |s| &s.0);
    /// ```
    pub fn project<C>(source: SourceSignal<&'static C>, part: fn(&C) -> &T) -> Self
    where
        C: Component,
    {
        Self(SignalKind::Source(Arc::new(move |store| {
            source.get(store).map(part)
        })))
    }

    /// Read the signal, subscribing the node currently being evaluated to it.
    ///
    /// Call it from a derived closure or an effect body and that node re-runs
    /// whenever the signal changes. A [`Value`](Signal::value) records no read
    /// and can never fail.
    pub fn get<'a>(&'a self, store: &'a SignalStore) -> Result<SignalRef<'a, T>, ReactError> {
        match &self.0 {
            SignalKind::Value(value) => Ok(SignalRef::Borrowed(value)),
            SignalKind::Cell(cell) => Ok(SignalRef::Cell(cell.read(store))),
            SignalKind::Source(read) => read(store).map(SignalRef::Borrowed),
        }
    }

    /// Whether reading this signal can ever wake anything.
    ///
    /// For a caller that would rather skip building machinery than build it and
    /// have it never fire — which is what [`watch_signal`] does with a static
    /// binding.
    ///
    /// [`watch_signal`]: super::source::SourceSignalView::watch_signal
    pub fn is_reactive(&self) -> bool {
        !matches!(self.0, SignalKind::Value(_))
    }

    /// The value, if this signal is a constant and so needs no world to read.
    pub fn as_value(&self) -> Option<&T> {
        match &self.0 {
            SignalKind::Value(value) => Some(value),
            _ => None,
        }
    }
}

impl<T: Clone + Send + Sync + 'static> Signal<T> {
    /// [`get`](Self::get), cloned, for a caller that would rather not hold a
    /// cell's read guard — which is most of them, since holding one across a
    /// write to the same cell would deadlock.
    pub fn cloned(&self, store: &SignalStore) -> Result<T, ReactError> {
        self.get(store).map(|value| value.clone())
    }
}

impl<T: Send + Sync + 'static> From<Cell<T>> for Signal<T> {
    fn from(cell: Cell<T>) -> Self {
        Self(SignalKind::Cell(cell))
    }
}

impl<C: Component> From<SourceSignal<&'static C>> for Signal<C> {
    fn from(source: SourceSignal<&'static C>) -> Self {
        Self(SignalKind::Source(Arc::new(move |store| source.get(store))))
    }
}

/// So a fixed entity can be passed anywhere a `Signal<Entity>` is wanted.
///
/// `Entity` cannot be a `Component`, so this is the only way to spell a constant
/// one — and it is what lets [`watch_signal`](
/// super::source::SourceSignalView::watch_signal) accept a literal without the
/// caller reaching for [`Signal::value`].
impl From<Entity> for Signal<Entity> {
    fn from(entity: Entity) -> Self {
        Self::value(entity)
    }
}

/// A borrow of a signal's value.
///
/// Two shapes because a cell's value lives behind a lock whose guard has to
/// outlive the read, where every other variant is already a plain borrow.
pub enum SignalRef<'a, T> {
    Borrowed(&'a T),
    Cell(CellRef<'a, T>),
}

impl<T> Deref for SignalRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            SignalRef::Borrowed(value) => value,
            SignalRef::Cell(value) => value,
        }
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for SignalRef<'_, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: PartialEq> PartialEq for SignalRef<'_, T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
