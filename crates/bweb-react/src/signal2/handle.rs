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
use bevy_query_observer::observer::{RetargetQueryObserver, TriggerQueryObserver};
use std::sync::{Arc, Mutex, RwLock, Weak};

use super::error::{SignalError, SignalReadGuard, SignalResult};
use super::graph::{PendingDirty, spawn_effect_with_source};
use super::insert::bind_sink;
use super::reactive_context::ReactiveContext;

/// The graph-node entity a handle refers to.
#[derive(Clone)]
pub(crate) struct SignalInner {
    pub(crate) entity: Entity,
}

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

    /// The graph node backing this signal, if any. `None` (the conservative
    /// default) disables edge pre-registration, keeping deferred sinks on the
    /// eager path.
    #[doc(hidden)]
    fn source_node(&self) -> Option<Entity> {
        None
    }

    /// Whether a read would currently succeed, **without** registering the read
    /// as a dependency of the evaluating node (unlike [`read`](Self::read),
    /// which registers even when `NotReady`).
    #[doc(hidden)]
    fn peek_ready(&self) -> bool {
        true
    }
}

/// Shared read implementation: register the source, then guard the value.
pub(crate) fn read_value<O>(
    entity: Entity,
    value: &RwLock<Option<O>>,
) -> SignalResult<SignalReadGuard<'_, O>> {
    // Register before the readiness check so a not-yet-ready read still subscribes.
    ReactiveContext::register(entity);
    let guard = value.read().unwrap();
    if guard.is_none() {
        return Err(SignalError::NotReady);
    }
    Ok(SignalReadGuard::Locked(guard))
}

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

    fn source_node(&self) -> Option<Entity> {
        Some(self.inner.entity)
    }

    fn peek_ready(&self) -> bool {
        self.value.read().unwrap().is_some()
    }
}

/// Which entity an [`ObserverSignal`]'s query observer watches. Resolved once,
/// during the deferred finalization queued by `SignalExt::signal`.
pub(crate) enum WatchTarget {
    /// Watch every entity matching the query (the default).
    Global,
    /// Watch a single, already-known entity ([`ObserverSignal::watch_entity`]).
    Entity(Entity),
    /// Watch the entity this signal is inserted into as a bundle; finalization is
    /// deferred to [`WatchBundle`]'s insertion hook ([`ObserverSignal::watch_bundle`]).
    Bundle,
    /// Watch the entity another signal currently points at
    /// ([`ObserverSignal::watch`]). The payload spawns the rebinder effect;
    /// finalization takes and runs it.
    Dynamic(Option<RebinderSpawn>),
}

/// Spawns the rebinder effect for a dynamically-watched signal. Consumed once by
/// the deferred finalization.
pub(crate) type RebinderSpawn = Box<dyn FnOnce(&mut World) + Send + Sync>;

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

    fn source_node(&self) -> Option<Entity> {
        Some(self.inner.entity)
    }

    fn peek_ready(&self) -> bool {
        self.shared.value.read().unwrap().is_some()
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

    /// Watch the entity `source` currently points at — a **dynamic** target for
    /// cross-entity chains ("component `T` on whichever entity that signal
    /// resolves to", e.g. a `DocLabel` on an object's prototype).
    ///
    /// A rebinder effect subscribes to `source`: whenever it yields a new
    /// entity, the query observer is re-targeted (the user system stays put;
    /// only the trampoline observers rebuild) and re-seeded from the new
    /// entity's current state. While `source` is `NotReady` the observer is
    /// unbound and this signal reads as `NotReady`. The rebinder holds `source`
    /// alive and is torn down with this signal's node.
    pub fn watch<S>(self, source: S) -> Self
    where
        S: SignalRead<Value = Entity>,
    {
        let node = self.inner.entity;
        let weak = Arc::downgrade(&self.shared);
        let spawn: RebinderSpawn = Box::new(move |world: &mut World| {
            spawn_rebinder(world, node, weak, source);
        });
        *self.shared.watch.lock().unwrap() = WatchTarget::Dynamic(Some(spawn));
        self
    }
}

/// Marker for the [`bind_sink`] binding that roots a dynamic watch's rebinder to
/// its signal node.
struct DynamicWatch;

/// Spawns the rebinder effect for [`ObserverSignal::watch`] and roots it to the
/// signal node (despawning the node tears the rebinder down via its binding).
///
/// The rebinder captures `source` **strongly** (the watcher keeps its target
/// signal alive) but only a `Weak` of the observer's shared state, so it never
/// pins the signal node against garbage collection.
fn spawn_rebinder<S, O>(world: &mut World, node: Entity, weak: Weak<ObserverShared<O>>, source: S)
where
    S: SignalRead<Value = Entity>,
    O: Send + Sync + 'static,
{
    let mut bound: Option<Entity> = None;
    let mut commands = world.commands();

    let hint = source.clone();
    let rebinder = spawn_effect_with_source(&mut commands, &hint, move |commands: &mut Commands| {
        // Read inside the effect so the rebinder is rewired as a subscriber of
        // `source` (even a `NotReady` read registers).
        match source.get() {
            Ok(target) if bound != Some(target) => {
                bound = Some(target);
                let weak = weak.clone();
                commands.queue(move |world: &mut World| {
                    let Some(shared) = weak.upgrade() else {
                        return;
                    };
                    if shared.builder.lock().unwrap().is_some() {
                        // First resolution: the one-shot builder constructs the
                        // observer bound to the target and seeds from it.
                        build_observer(&shared, world, Some(target));
                    } else {
                        world.retarget_query_observer(node, &[target]);
                        // Re-seed from the new target's current state.
                        world.trigger_query_observer(node, target);
                    }
                });
            }
            Ok(_) => {}
            Err(SignalError::NotReady) => {
                // Source lost its target: unbind and read as NotReady until a
                // new target resolves.
                if bound.take().is_some() {
                    let weak = weak.clone();
                    commands.queue(move |world: &mut World| {
                        let Some(shared) = weak.upgrade() else {
                            return;
                        };
                        world.retarget_query_observer(node, &[]);
                        *shared.value.write().unwrap() = None;
                        world.resource_mut::<PendingDirty>().0.push(node);
                    });
                }
            }
        }
    });

    bind_sink::<DynamicWatch>(&mut commands, node, rebinder);
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

/// A unified signal source: a query-observer input, a derived value, or a
/// constant. Implements [`SignalRead`] (and therefore, via the blanket impl,
/// [`SignalMap`](super::SignalMap)), so any of the three can be read
/// interchangeably wherever a signal is expected — e.g. as the value behind a
/// widget. Construct via [`Signal::Static`] or `.into()` from a
/// [`DerivedSignal`] / [`ObserverSignal`].
pub enum Signal<O> {
    /// A query-observer-driven input ([`ObserverSignal`]).
    Signal(ObserverSignal<O>),
    /// A derived / memoized / polled value ([`DerivedSignal`]).
    Derived(DerivedSignal<O>),
    /// A constant that never changes and registers no reactive dependency.
    Static(O),
}

// Hand-written so the handle variants clone without `O: Clone`; only `Static`
// needs it, and the enum as a whole requires it anyway to satisfy `SignalRead`.
impl<O: Clone> Clone for Signal<O> {
    fn clone(&self) -> Self {
        match self {
            Self::Signal(signal) => Self::Signal(signal.clone()),
            Self::Derived(signal) => Self::Derived(signal.clone()),
            Self::Static(value) => Self::Static(value.clone()),
        }
    }
}

impl<O: Clone + Send + Sync + 'static> SignalRead for Signal<O> {
    type Value = O;

    fn read(&self) -> SignalResult<SignalReadGuard<'_, O>> {
        match self {
            Self::Signal(signal) => signal.read(),
            Self::Derived(signal) => signal.read(),
            Self::Static(value) => Ok(SignalReadGuard::Borrowed(value)),
        }
    }

    fn source_node(&self) -> Option<Entity> {
        match self {
            Self::Signal(signal) => signal.source_node(),
            Self::Derived(signal) => signal.source_node(),
            Self::Static(_) => None,
        }
    }

    fn peek_ready(&self) -> bool {
        match self {
            Self::Signal(signal) => signal.peek_ready(),
            Self::Derived(signal) => signal.peek_ready(),
            Self::Static(_) => true,
        }
    }
}

impl<O> From<ObserverSignal<O>> for Signal<O> {
    fn from(signal: ObserverSignal<O>) -> Self {
        Self::Signal(signal)
    }
}

impl<O> From<DerivedSignal<O>> for Signal<O> {
    fn from(signal: DerivedSignal<O>) -> Self {
        Self::Derived(signal)
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
