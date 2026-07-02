//! Reactive Bundle insertion: dropping a signal (or a mapped / optional view of
//! one) onto an entity keeps a computed bundle in sync with the signal, cleaning
//! up the prior value and tearing the sink down on replacement/despawn.

use bevy_ecs::{
    component::{Mutable, StorageType},
    lifecycle::{ComponentHook, HookContext},
    prelude::*,
    world::DeferredWorld,
};
use std::marker::PhantomData;
use std::sync::Arc;

use crate::cleanup::ReactiveCleanupExt;

use super::error::SignalError;
use super::graph::{despawn_node, spawn_effect};
use super::handle::{DerivedSignal, ObserverSignal, SignalRead};

/// Spawns a sink that reads `source` by reference, maps it through `f`, and
/// reactively (re)inserts the resulting `O2` bundle into `host`, cleaning up the
/// prior value. Skips insertion while the source is [`SignalError::NotReady`],
/// leaving whatever was last inserted.
fn spawn_map_sink<S, F, O2>(commands: &mut Commands, host: Entity, source: S, f: Arc<F>) -> Entity
where
    S: SignalRead,
    F: Fn(&S::Value) -> O2 + Send + Sync + 'static,
    O2: Bundle + Send + Sync + 'static,
{
    spawn_effect(commands, move |mut commands: Commands| {
        match source.read() {
            Ok(guard) => {
                let bundle = f(&guard);
                commands
                    .entity(host)
                    .reactive_cleanup::<O2>()
                    .try_insert(bundle);
            }
            Err(SignalError::NotReady) => {}
        }
    })
}

/// Like [`spawn_map_sink`] but the mapper also receives the sink's `Commands`, so
/// it can spawn child signals / entities while producing the bundle. Backs
/// [`SignalMap::map_commands`].
fn spawn_map_commands_sink<S, F, O2>(
    commands: &mut Commands,
    host: Entity,
    source: S,
    f: Arc<F>,
) -> Entity
where
    S: SignalRead,
    F: Fn(&mut Commands, &S::Value) -> O2 + Send + Sync + 'static,
    O2: Bundle + Send + Sync + 'static,
{
    spawn_effect(commands, move |mut commands: Commands| {
        match source.read() {
            Ok(guard) => {
                let bundle = f(&mut commands, &guard);
                commands
                    .entity(host)
                    .reactive_cleanup::<O2>()
                    .try_insert(bundle);
            }
            Err(SignalError::NotReady) => {}
        }
    })
}

type SpawnSink = Arc<dyn Fn(&mut Commands, Entity) -> Entity + Send + Sync>;

/// A type-erased reactive insertion. Dropped onto a host entity it spawns a sink
/// that (re)inserts a `K` bundle derived from some source signal. Produced by
/// [`SignalMap::map`] ([`MappedSignal`]) and [`SignalOption::option`]
/// ([`OptionSignal`]).
pub struct ReactiveInsert<K> {
    spawn: SpawnSink,
    _marker: PhantomData<fn() -> K>,
}

/// A signal value reactively mapped into an insertable bundle. See
/// [`SignalMap::map`].
pub type MappedSignal<O> = ReactiveInsert<O>;

/// An `Option`-valued signal reactively inserted as a bundle, removing the
/// bundle when the value is `None`. See [`SignalOption::option`].
pub type OptionSignal<O> = ReactiveInsert<O>;

impl<K> Clone for ReactiveInsert<K> {
    fn clone(&self) -> Self {
        Self {
            spawn: self.spawn.clone(),
            _marker: PhantomData,
        }
    }
}

impl<K: Send + Sync + 'static> Component for ReactiveInsert<K> {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;

    fn on_insert() -> Option<ComponentHook> {
        fn hook<K: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
            let spawn = world
                .get::<ReactiveInsert<K>>(ctx.entity)
                .unwrap()
                .spawn
                .clone();
            let host = ctx.entity;
            let mut commands = world.commands();
            let sink = spawn(&mut commands, host);
            bind_sink::<K>(&mut commands, host, sink);
        }
        Some(hook::<K>)
    }

    fn on_replace() -> Option<ComponentHook> {
        Some(unbind_sink::<K>)
    }
}

/// Reactively map a signal's value **by reference** into an insertable bundle.
pub trait SignalMap: SignalRead {
    /// Maps this signal's value into a [`MappedSignal`] component: dropped onto an
    /// entity it (re)inserts `f`'s output whenever this signal changes.
    fn map<F, O2>(&self, f: F) -> MappedSignal<O2>
    where
        Self: Sized,
        F: Fn(&Self::Value) -> O2 + Send + Sync + 'static,
        O2: Bundle + Send + Sync + 'static,
    {
        let source = self.clone();
        let f = Arc::new(f);
        ReactiveInsert {
            spawn: Arc::new(move |commands: &mut Commands, host: Entity| {
                spawn_map_sink(commands, host, source.clone(), f.clone())
            }),
            _marker: PhantomData,
        }
    }

    /// Like [`map`](Self::map), but the mapper also receives `&mut Commands`, so it
    /// can spawn child signals / entities while producing the bundle. This is the
    /// signal2 equivalent of the old framework's `map_system` — use it for a
    /// reactive branch that renders a subtree (e.g. building a nested list or
    /// per-variant widget) rather than a plain value.
    fn map_commands<F, O2>(&self, f: F) -> MappedSignal<O2>
    where
        Self: Sized,
        F: Fn(&mut Commands, &Self::Value) -> O2 + Send + Sync + 'static,
        O2: Bundle + Send + Sync + 'static,
    {
        let source = self.clone();
        let f = Arc::new(f);
        ReactiveInsert {
            spawn: Arc::new(move |commands: &mut Commands, host: Entity| {
                spawn_map_commands_sink(commands, host, source.clone(), f.clone())
            }),
            _marker: PhantomData,
        }
    }
}

impl<S: SignalRead> SignalMap for S {}

/// Reactive-insertion `Component` for a signal handle whose value `O` is itself a
/// bundle: dropped onto an entity, it keeps the entity's `O` in sync with the
/// signal.
macro_rules! impl_bundle_component {
    ($handle:ident) => {
        impl<O: Bundle + Clone + Send + Sync + 'static> Component for $handle<O> {
            const STORAGE_TYPE: StorageType = StorageType::Table;
            type Mutability = Mutable;

            fn on_insert() -> Option<ComponentHook> {
                Some(handle_on_insert::<$handle<O>, O>)
            }

            fn on_replace() -> Option<ComponentHook> {
                Some(unbind_sink::<O>)
            }
        }
    };
}

impl_bundle_component!(DerivedSignal);
impl_bundle_component!(ObserverSignal);

/// `on_insert` hook shared by the signal-handle `Component` impls: spawn an
/// identity sink (insert the value's clone) and record the binding for teardown.
fn handle_on_insert<S, O>(mut world: DeferredWorld, ctx: HookContext)
where
    S: SignalRead<Value = O> + Component,
    O: Bundle + Clone + Send + Sync + 'static,
{
    let source = world.get::<S>(ctx.entity).unwrap().clone();
    let host = ctx.entity;
    let mut commands = world.commands();
    let sink = spawn_map_sink(
        &mut commands,
        host,
        source,
        Arc::new(|value: &O| value.clone()),
    );
    bind_sink::<O>(&mut commands, host, sink);
}

/// Records the sink entity created for a reactive insertion of bundle `K` on a
/// host, so it can be torn down when the insertion is replaced or the host is
/// despawned.
struct ReactiveBinding<K>(Entity, PhantomData<fn() -> K>);

impl<K: Send + Sync + 'static> Component for ReactiveBinding<K> {
    const STORAGE_TYPE: StorageType = StorageType::Table;
    type Mutability = Mutable;

    fn on_replace() -> Option<ComponentHook> {
        fn hook<K: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
            if let Some(sink) = world.get::<ReactiveBinding<K>>(ctx.entity).map(|b| b.0) {
                world
                    .commands()
                    .queue(move |world: &mut World| despawn_node(world, sink));
            }
        }
        Some(hook::<K>)
    }
}

pub(crate) fn bind_sink<K: Send + Sync + 'static>(
    commands: &mut Commands,
    host: Entity,
    sink: Entity,
) {
    commands
        .entity(host)
        .insert(ReactiveBinding::<K>(sink, PhantomData));
}

/// `on_replace` hook: drop the host's [`ReactiveBinding`] (which despawns the
/// sink). Guarded so it's a no-op if the host is already gone.
fn unbind_sink<K: Send + Sync + 'static>(mut world: DeferredWorld, ctx: HookContext) {
    let host = ctx.entity;
    world.commands().queue(move |world: &mut World| {
        if let Ok(mut entity) = world.get_entity_mut(host) {
            entity.remove::<ReactiveBinding<K>>();
        }
    });
}

/// Reactive insertion for `Option`-valued signals: inserts the inner bundle when
/// `Some`, removes it when `None`, and leaves it untouched while `NotReady`.
pub trait SignalOption {
    type Item;

    fn option(&self) -> OptionSignal<Self::Item>;
}

impl<S, T> SignalOption for S
where
    S: SignalRead<Value = Option<T>>,
    T: Bundle + Clone + Send + Sync + 'static,
{
    type Item = T;

    fn option(&self) -> OptionSignal<T> {
        let source = self.clone();
        ReactiveInsert {
            spawn: Arc::new(move |commands: &mut Commands, host: Entity| {
                let source = source.clone();
                spawn_effect(commands, move |mut commands: Commands| {
                    match source.read() {
                        Ok(guard) => match &*guard {
                            Some(value) => {
                                let value = value.clone();
                                commands
                                    .entity(host)
                                    .reactive_cleanup::<T>()
                                    .try_insert(value);
                            }
                            None => {
                                commands.entity(host).remove::<T>();
                            }
                        },
                        Err(SignalError::NotReady) => {}
                    }
                })
            }),
            _marker: PhantomData,
        }
    }
}
