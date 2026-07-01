//! Signal handles and the fallible read API.
//!
//! [`DerivedSignal`] and [`ObserverSignal`] are cheap, cloneable handles onto a
//! graph node's value cell; [`SignalRead`] is the shared read surface. The
//! observer-input machinery ([`ObserverShared`], [`WatchTarget`], [`WatchBundle`])
//! also lives here, since it is intrinsic to what an [`ObserverSignal`] *is*.

use bevy_ecs::{
    component::{Mutable, StorageType},
    lifecycle::{ComponentHook, HookContext},
    prelude::*,
    world::DeferredWorld,
};
use std::sync::{Arc, Mutex, RwLock};

use super::error::{SignalError, SignalReadGuard, SignalResult};
use super::reactive_context::ReactiveContext;

/// The graph-node entity a handle refers to.
#[derive(Clone)]
pub(crate) struct SignalInner {
    pub(crate) entity: Entity,
}

// ---------------------------------------------------------------------------
// Read API
// ---------------------------------------------------------------------------

/// Fallible read access shared by every signal handle.
///
/// Reads register the signal as a source of whatever node is currently
/// evaluating (via [`ReactiveContext`]), and error with [`SignalError::NotReady`]
/// until the signal has produced a value.
pub trait SignalRead: Clone + Send + Sync + 'static {
    type Value;

    /// Reads the current value by reference.
    fn read(&self) -> SignalResult<SignalReadGuard<'_, Self::Value>>;

    /// Reads and clones the current value.
    fn get(&self) -> SignalResult<Self::Value>
    where
        Self::Value: Clone,
    {
        Ok(self.read()?.clone())
    }
}

/// Shared read implementation: register the source, then guard the value.
fn read_value<O>(
    entity: Entity,
    value: &RwLock<Option<O>>,
) -> SignalResult<SignalReadGuard<'_, O>> {
    // Register before the readiness check so a not-yet-ready read still subscribes.
    ReactiveContext::register(entity);
    let guard = value.read().unwrap();
    if guard.is_none() {
        return Err(SignalError::NotReady);
    }
    Ok(SignalReadGuard(guard))
}

// ---------------------------------------------------------------------------
// DerivedSignal
// ---------------------------------------------------------------------------

/// A handle onto a `derive`/`memo` node's value.
pub struct DerivedSignal<O> {
    pub(crate) inner: SignalInner,
    pub(crate) value: Arc<RwLock<Option<O>>>,
}

impl<O> Clone for DerivedSignal<O> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            value: self.value.clone(),
        }
    }
}

impl<O: Send + Sync + 'static> SignalRead for DerivedSignal<O> {
    type Value = O;

    fn read(&self) -> SignalResult<SignalReadGuard<'_, O>> {
        read_value(self.inner.entity, &self.value)
    }
}

// ---------------------------------------------------------------------------
// ObserverSignal + watch machinery
// ---------------------------------------------------------------------------

/// Which entity an [`ObserverSignal`]'s query observer watches. Resolved once,
/// during the deferred finalization queued by `Signal::signal`.
pub(crate) enum WatchTarget {
    /// Watch every entity matching the query (the default).
    Global,
    /// Watch a single, already-known entity ([`ObserverSignal::watch_entity`]).
    Entity(Entity),
    /// Watch the entity this signal is inserted into as a bundle; finalization is
    /// deferred to [`WatchBundle`]'s insertion hook ([`ObserverSignal::watch_bundle`]).
    Bundle,
}

/// Spawns the backing query observer, watching `Some(entity)` or, if `None`,
/// every matching entity. Consumed exactly once.
pub(crate) type ObserverBuilder = Box<dyn FnOnce(&mut World, Option<Entity>) + Send + Sync>;

/// State shared between an [`ObserverSignal`]'s clones: its value cell, the
/// one-shot observer builder, and the pending watch target.
pub(crate) struct ObserverShared<O> {
    pub(crate) value: RwLock<Option<O>>,
    pub(crate) builder: Mutex<Option<ObserverBuilder>>,
    pub(crate) watch: Mutex<WatchTarget>,
}

/// A handle onto a query-observer-driven input signal.
pub struct ObserverSignal<O> {
    pub(crate) inner: SignalInner,
    pub(crate) shared: Arc<ObserverShared<O>>,
}

// Hand-written (not derived) so cloning a handle never requires `O: Clone` — the
// only `O` lives behind an `Arc`.
impl<O> Clone for ObserverSignal<O> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            shared: self.shared.clone(),
        }
    }
}

impl<O: Send + Sync + 'static> SignalRead for ObserverSignal<O> {
    type Value = O;

    fn read(&self) -> SignalResult<SignalReadGuard<'_, O>> {
        read_value(self.inner.entity, &self.shared.value)
    }
}

impl<O: Send + Sync + 'static> ObserverSignal<O> {
    /// Watch a specific entity: the observer fires only for `entity`.
    pub fn watch_entity(self, entity: Entity) -> Self {
        *self.shared.watch.lock().unwrap() = WatchTarget::Entity(entity);
        self
    }

    /// Watch the entity this signal is inserted into. Returns a [`WatchBundle`]
    /// component: drop it into the target entity's bundle and the observer will
    /// watch that entity (useful when the entity doesn't exist yet at signal
    /// creation, e.g. inside a returned `impl Bundle`).
    pub fn watch_bundle(&self) -> WatchBundle<O> {
        *self.shared.watch.lock().unwrap() = WatchTarget::Bundle;
        WatchBundle {
            shared: self.shared.clone(),
        }
    }
}

/// Runs an [`ObserverShared`]'s one-shot observer builder with the resolved watch
/// target. A no-op if the builder was already consumed.
pub(crate) fn build_observer<O>(
    shared: &ObserverShared<O>,
    world: &mut World,
    watched: Option<Entity>,
) {
    if let Some(builder) = shared.builder.lock().unwrap().take() {
        builder(world, watched);
    }
}

/// Wires an [`ObserverSignal`]'s query observer to the entity it is inserted
/// into. Produced by [`ObserverSignal::watch_bundle`].
pub struct WatchBundle<O> {
    shared: Arc<ObserverShared<O>>,
}

impl<O: Send + Sync + 'static> Component for WatchBundle<O> {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;

    fn on_insert() -> Option<ComponentHook> {
        fn hook<O: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
            let host = ctx.entity;
            let shared = world.get::<WatchBundle<O>>(host).unwrap().shared.clone();
            world
                .commands()
                .queue(move |world: &mut World| build_observer(&shared, world, Some(host)));
        }
        Some(hook::<O>)
    }
}
