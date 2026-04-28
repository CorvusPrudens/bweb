use bevy_app::prelude::*;
use bevy_ecs::{
    lifecycle::HookContext,
    prelude::*,
    system::{EntityCommands, IntoSystem, ReadOnlySystem},
    world::DeferredWorld,
};
use std::{
    any::Any,
    marker::PhantomData,
    sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard, atomic::AtomicU32},
};

pub mod observer;
pub mod reactive_observer;
pub mod rw_signal;
pub mod signal_query;
pub mod signal_res;
pub mod signal_world;
mod system;
pub mod traits;

use crate::{
    cleanup::ReactiveCleanupExt, prelude::ReadSignal, signal::system::SignalStateContainer,
};

pub(crate) struct SignalPlugin;

impl Plugin for SignalPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.add_plugins(system::SystemPlugin)
            .init_resource::<SweepFrequency>()
            .init_resource::<reactive_observer::RemovedSet>()
            .add_systems(
                crate::ReactSchedule,
                reactive_observer::RemovedSet::update
                    .before(crate::ReactScheduleSystems::EvaluateSignals),
            )
            .add_systems(Last, gc_pass);
    }
}

#[derive(Component, Default)]
pub struct SignalMarker;

#[derive(Resource)]
pub struct SweepFrequency(pub core::time::Duration);

impl Default for SweepFrequency {
    fn default() -> Self {
        Self(core::time::Duration::from_secs(1))
    }
}

pub trait SignalTick {
    fn tick(&self) -> u32;
}

/// The inner value of a signal, with a tick
/// for indicating whether it's changed.
struct SignalInner<T> {
    value: RwLock<Option<T>>,
    tick: AtomicU32,
}

impl<T> SignalInner<T> {
    pub fn new(value: Option<T>) -> Self {
        Self {
            value: RwLock::new(value),
            tick: AtomicU32::new(0),
        }
    }

    pub fn set_inner(&self, value: T) {
        let mut writer = self.value.write().unwrap();
        *writer = Some(value);
        self.tick.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

impl<T> SignalTick for SignalInner<T> {
    fn tick(&self) -> u32 {
        self.tick.load(std::sync::atomic::Ordering::Relaxed)
    }
}

pub struct SignalReadGuard<'a, T>(RwLockReadGuard<'a, Option<T>>);

impl<'a, T> SignalReadGuard<'a, T> {
    fn new(guard: RwLockReadGuard<'a, Option<T>>) -> Self {
        if guard.is_none() {
            panic!("Attempted to create a signal read guard for a signal without a value");
        }

        Self(guard)
    }
}

impl<T> core::ops::Deref for SignalReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // NOTE: this cannot panic since we check that the inner
        // value `is_some` at construction.
        self.0.as_ref().unwrap()
    }
}

pub struct SignalWriteGuard<'a, T> {
    data: RwLockWriteGuard<'a, Option<T>>,
    tick: &'a AtomicU32,
}

impl<'a, T> SignalWriteGuard<'a, T> {
    fn new(guard: RwLockWriteGuard<'a, Option<T>>, tick: &'a AtomicU32) -> Self {
        if guard.is_none() {
            panic!("Attempted to create a signal read guard for a signal without a value");
        }

        Self { data: guard, tick }
    }
}

impl<T> core::ops::Deref for SignalWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // NOTE: this cannot panic since we check that the inner
        // value `is_some` at construction.
        self.data.as_ref().unwrap()
    }
}

impl<T> core::ops::DerefMut for SignalWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.tick.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // NOTE: this cannot panic since we check that the inner
        // value `is_some` at construction.
        self.data.as_mut().unwrap()
    }
}

impl<T> traits::Read for SignalInner<T> {
    type Guard<'a>
        = SignalReadGuard<'a, T>
    where
        Self: 'a;

    type Value = T;

    fn try_read(&self) -> Option<Self::Guard<'_>> {
        self.value.read().ok().map(SignalReadGuard::new)
    }
}

impl<T> traits::Write for SignalInner<T> {
    type Guard<'a>
        = SignalWriteGuard<'a, T>
    where
        Self: 'a;

    type Value = T;

    fn try_write(&self) -> Option<Self::Guard<'_>> {
        self.value
            .write()
            .ok()
            .map(|g| SignalWriteGuard::new(g, &self.tick))
    }
}

#[derive(Component)]
#[require(SignalMarker)]
struct SignalGc {
    signal: Arc<dyn SignalTick + Send + Sync>,
    rest_strong_count: usize,
}

impl SignalGc {
    pub fn new<S: SignalTick + Send + Sync + 'static>(
        signal: &Arc<S>,
        rest_strong_count: usize,
    ) -> Self {
        Self {
            signal: Arc::clone(signal) as Arc<dyn SignalTick + Send + Sync>,
            rest_strong_count,
        }
    }
}

pub struct DerivedSignal<T> {
    data: Arc<SignalInner<T>>,
    entity: Entity,
}

impl<T: Clone + Bundle> DerivedSignal<T> {
    fn inserter(any: &dyn Any, mut commands: EntityCommands) {
        let value = any.downcast_ref::<T>().unwrap().clone();
        commands.reactive_cleanup::<T>().insert(value);
    }
}

impl<T> bevy_ecs::component::Component for DerivedSignal<T>
where
    Self: Send + Sync + 'static,
    T: Clone + Bundle,
{
    const STORAGE_TYPE: bevy_ecs::component::StorageType = bevy_ecs::component::StorageType::Table;
    type Mutability = bevy_ecs::component::Mutable;

    fn register_required_components(
        _requiree: bevy_ecs::component::ComponentId,
        _required_components: &mut bevy_ecs::component::RequiredComponentsRegistrator,
    ) {
    }

    fn clone_behavior() -> bevy_ecs::component::ComponentCloneBehavior {
        use bevy_ecs::component::DefaultCloneBehaviorViaClone;
        (&&&bevy_ecs::component::DefaultCloneBehaviorSpecialization::<Self>::default())
            .default_clone_behavior()
    }

    fn on_insert() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        fn on_insert<T: Clone + Bundle>(mut world: DeferredWorld, context: HookContext) {
            let state = world
                .get::<DerivedSignal<T>>(context.entity)
                .unwrap()
                .entity;

            if let Some(mut state) = world.get_mut::<SignalStateContainer>(state) {
                state.insert(
                    system::Inserter::Ptr(DerivedSignal::<T>::inserter),
                    context.entity,
                );
            }
        }

        Some(on_insert::<T>)
    }

    fn on_replace() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        fn on_replace<T: Clone + Bundle>(mut world: DeferredWorld, context: HookContext) {
            let state = world
                .get::<DerivedSignal<T>>(context.entity)
                .unwrap()
                .entity;

            if let Some(mut state) = world.get_mut::<SignalStateContainer>(state) {
                state.remove(context.entity);
            }
        }

        Some(on_replace::<T>)
    }
}

impl<T> Clone for DerivedSignal<T> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
            entity: self.entity,
        }
    }
}

impl<T: Send + Sync + 'static> traits::Read for DerivedSignal<T> {
    type Guard<'a>
        = SignalReadGuard<'a, T>
    where
        Self: 'a;
    type Value = T;

    fn try_read(&self) -> Option<Self::Guard<'_>> {
        if let Some(observer) = reactive_observer::SignalObserver::get() {
            observer.add_signal(reactive_observer::SignalSubscriber::new(&self.data));
        }

        self.data.try_read()
    }
}

#[derive(Clone)]
pub enum Signal<T> {
    Derived(DerivedSignal<T>),
    ReadSignal(ReadSignal<T>),
    Static(T),
}

impl<T> From<DerivedSignal<T>> for Signal<T> {
    fn from(value: DerivedSignal<T>) -> Self {
        Self::Derived(value)
    }
}

impl<T> From<ReadSignal<T>> for Signal<T> {
    fn from(value: ReadSignal<T>) -> Self {
        Self::ReadSignal(value)
    }
}

impl<T> From<T> for Signal<T> {
    fn from(value: T) -> Self {
        Self::Static(value)
    }
}

impl<T: Send + Sync + 'static> traits::Read for Signal<T> {
    type Guard<'a>
        = SignalEnumReadGuard<'a, T>
    where
        Self: 'a;
    type Value = T;

    fn try_read(&self) -> Option<Self::Guard<'_>> {
        match self {
            Self::Derived(d) => Some(SignalEnumReadGuard::Signal(d.try_read()?)),
            Self::ReadSignal(r) => Some(SignalEnumReadGuard::Signal(r.try_read()?)),
            Self::Static(s) => Some(SignalEnumReadGuard::Static(s)),
        }
    }
}

pub enum SignalEnumReadGuard<'a, T> {
    Signal(SignalReadGuard<'a, T>),
    Static(&'a T),
}

impl<T> core::ops::Deref for SignalEnumReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Signal(d) => d.deref(),
            Self::Static(d) => d,
        }
    }
}

impl<O: Clone + Send + Sync + 'static> DerivedSignal<O> {
    pub fn new<S, M>(mut commands: Commands, system: S) -> Self
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
    {
        let inner = Arc::new(SignalInner {
            value: RwLock::new(None),
            tick: AtomicU32::new(0),
        });

        let system = system::DerivedSystem::new(system, Arc::clone(&inner));
        let system = commands.register_system(system);
        commands.entity(system.entity()).insert(SignalMarker);

        let set = reactive_observer::SubscriberSet::new();

        let id = commands
            .spawn((
                set.clone(),
                SignalStateContainer::new(system, system::SignalOutputs::new()),
                SignalGc::new(&inner, 2),
            ))
            .id();

        commands.queue(move |world: &mut World| {
            system::evaluate_signal(world, id, true);
        });

        DerivedSignal {
            data: inner,
            entity: id,
        }
    }
}

impl<O: PartialEq + Clone + Send + Sync + 'static> DerivedSignal<O> {
    pub fn memo<S, M>(mut commands: Commands, system: S) -> Self
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        S::System: ReadOnlySystem,
    {
        let inner = Arc::new(SignalInner {
            value: RwLock::new(None),
            tick: AtomicU32::new(0),
        });

        let system = system::MemoSystem::new(system, inner.clone());
        let system = commands.register_system(system);
        commands.entity(system.entity()).insert(SignalMarker);

        let id = commands
            .spawn((
                SignalStateContainer::new(system, system::SignalOutputs::new()),
                SignalGc::new(&inner, 2),
            ))
            .id();

        commands.queue(move |world: &mut World| {
            system::evaluate_memo(world, id);
        });

        DerivedSignal {
            data: inner,
            entity: id,
        }
    }
}

pub struct MappedSignal<O> {
    tick: Arc<dyn SignalTick + Send + Sync>,
    entity: Entity,
    mapper: system::InserterFn,
    output: PhantomData<fn() -> O>,
}

impl<T: Send + Sync + 'static> DerivedSignal<T> {
    pub fn map<F, O>(&self, mapper: F) -> MappedSignal<O>
    where
        F: Fn(&T) -> O + Send + Sync + 'static,
        O: Bundle,
    {
        MappedSignal {
            tick: Arc::clone(&self.data) as Arc<dyn SignalTick + Send + Sync>,
            entity: self.entity,
            mapper: Arc::new(move |any: &dyn Any, mut entity: EntityCommands| {
                let value = any.downcast_ref().unwrap();
                let output = mapper(value);
                entity.insert(output);
            }),
            output: PhantomData,
        }
    }
}

impl<T: Clone + Send + Sync + 'static> DerivedSignal<T> {
    pub fn map_system<S, O, M>(&self, mapper: S) -> MappedSignal<O>
    where
        S: IntoSystem<In<T>, O, M>,
        S::System: Send + Sync + 'static,
        O: Bundle,
    {
        let source_id = self.entity;

        let system = Mutex::new(Some(IntoSystem::into_system(mapper)));
        let system_id = Mutex::new(None);

        MappedSignal {
            tick: Arc::clone(&self.data) as Arc<dyn SignalTick + Send + Sync>,
            entity: self.entity,
            mapper: Arc::new(move |any: &dyn Any, mut entity: EntityCommands| {
                let target_id = entity.id();
                let value = any.downcast_ref::<T>().unwrap().clone();

                let mut commands = entity.commands();

                let id = system_id.lock().unwrap().as_ref().cloned();
                let id = match id {
                    Some(id) => id,
                    None => {
                        let system = system.lock().unwrap().take().unwrap();
                        let id = commands.register_system(system);
                        *system_id.lock().unwrap() = Some(id);

                        commands
                            .entity(id.entity())
                            .insert((ChildOf(source_id), SignalMarker));

                        id
                    }
                };

                commands.queue(move |world: &mut World| -> Result {
                    let value = world.run_system_with(id, value)?;
                    world
                        .get_entity_mut(target_id)?
                        .reactive_cleanup::<O>()
                        .insert(value);

                    Ok(())
                });
            }),
            output: PhantomData,
        }
    }
}

impl<O> Clone for MappedSignal<O> {
    fn clone(&self) -> Self {
        Self {
            tick: Arc::clone(&self.tick),
            entity: self.entity,
            mapper: Arc::clone(&self.mapper),
            output: self.output,
        }
    }
}

impl<O> bevy_ecs::component::Component for MappedSignal<O>
where
    Self: Send + Sync + 'static,
    O: Bundle,
{
    const STORAGE_TYPE: bevy_ecs::component::StorageType = bevy_ecs::component::StorageType::Table;
    type Mutability = bevy_ecs::component::Mutable;

    fn register_required_components(
        _requiree: bevy_ecs::component::ComponentId,
        _required_components: &mut bevy_ecs::component::RequiredComponentsRegistrator,
    ) {
    }

    fn clone_behavior() -> bevy_ecs::component::ComponentCloneBehavior {
        use bevy_ecs::component::DefaultCloneBehaviorViaClone;
        (&&&bevy_ecs::component::DefaultCloneBehaviorSpecialization::<Self>::default())
            .default_clone_behavior()
    }

    fn on_insert() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        fn on_insert<O: Bundle>(mut world: DeferredWorld, context: HookContext) {
            let signal = world.get::<MappedSignal<O>>(context.entity).unwrap();

            let state_entity = signal.entity;
            let mapper = Arc::clone(&signal.mapper);

            if let Some(mut state) = world.get_mut::<SignalStateContainer>(state_entity) {
                state.insert(system::Inserter::Arc(mapper), context.entity);
            }
        }

        Some(on_insert::<O>)
    }

    fn on_replace() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        fn on_replace<O: Bundle>(mut world: DeferredWorld, context: HookContext) {
            let state = world.get::<MappedSignal<O>>(context.entity).unwrap().entity;
            if let Some(mut state) = world.get_mut::<SignalStateContainer>(state) {
                state.remove(context.entity);
            }
        }

        Some(on_replace::<O>)
    }
}

pub struct OptionSignal<T> {
    tick: Arc<dyn SignalTick + Send + Sync>,
    entity: Entity,
    marker: PhantomData<T>,
}

impl<T: Send + Sync + 'static> OptionSignal<T> {
    pub fn map<F, O>(&self, mapper: F) -> MappedSignal<O>
    where
        F: Fn(&T) -> O + Send + Sync + 'static,
        O: Bundle,
    {
        MappedSignal {
            tick: Arc::clone(&self.tick),
            entity: self.entity,
            mapper: Arc::new(move |any: &dyn Any, mut entity: EntityCommands| {
                let value = any.downcast_ref::<Option<T>>().unwrap();

                match value {
                    Some(value) => {
                        let output = mapper(value);
                        entity.reactive_cleanup::<O>().insert(output);
                    }
                    None => {
                        entity.reactive_cleanup::<O>().remove_with_requires::<O>();
                    }
                }
            }),
            output: PhantomData,
        }
    }
}

impl<T: Clone + Send + Sync + 'static> OptionSignal<T> {
    pub fn map_system<S, O, M>(&self, mapper: S) -> MappedSignal<O>
    where
        S: IntoSystem<In<T>, O, M>,
        S::System: Send + Sync + 'static,
        O: Bundle,
    {
        let source_id = self.entity;

        let system = Mutex::new(Some(IntoSystem::into_system(mapper)));
        let system_id = Mutex::new(None);

        MappedSignal {
            tick: Arc::clone(&self.tick),
            entity: self.entity,
            mapper: Arc::new(move |any: &dyn Any, mut entity: EntityCommands| {
                let value = any.downcast_ref::<Option<T>>().unwrap();

                match value {
                    Some(value) => {
                        let target_id = entity.id();
                        let value = value.clone();

                        let mut commands = entity.commands();

                        let id = system_id.lock().unwrap().as_ref().cloned();
                        let id = match id {
                            Some(id) => id,
                            None => {
                                let system = system.lock().unwrap().take().unwrap();
                                let id = commands.register_system(system);
                                *system_id.lock().unwrap() = Some(id);

                                commands
                                    .entity(id.entity())
                                    .insert((ChildOf(source_id), SignalMarker));

                                id
                            }
                        };

                        commands.queue(move |world: &mut World| -> Result {
                            let value = world.run_system_with(id, value)?;
                            world
                                .get_entity_mut(target_id)?
                                .reactive_cleanup::<O>()
                                .insert(value);

                            Ok(())
                        });
                    }
                    None => {
                        entity.reactive_cleanup::<O>().remove_with_requires::<O>();
                    }
                }
            }),
            output: PhantomData,
        }
    }
}

impl<T> Clone for OptionSignal<T> {
    fn clone(&self) -> Self {
        Self {
            tick: Arc::clone(&self.tick),
            entity: self.entity,
            marker: self.marker,
        }
    }
}

impl<T: Send + Sync + 'static> DerivedSignal<Option<T>> {
    pub fn option(&self) -> OptionSignal<T> {
        OptionSignal {
            tick: Arc::clone(&self.data) as Arc<dyn SignalTick + Send + Sync>,
            entity: self.entity,
            marker: PhantomData,
        }
    }
}

impl<T> bevy_ecs::component::Component for OptionSignal<T>
where
    Self: Send + Sync + 'static,
    T: Bundle + Clone,
{
    const STORAGE_TYPE: bevy_ecs::component::StorageType = bevy_ecs::component::StorageType::Table;
    type Mutability = bevy_ecs::component::Mutable;

    fn register_required_components(
        _requiree: bevy_ecs::component::ComponentId,
        _required_components: &mut bevy_ecs::component::RequiredComponentsRegistrator,
    ) {
    }

    fn clone_behavior() -> bevy_ecs::component::ComponentCloneBehavior {
        use bevy_ecs::component::DefaultCloneBehaviorViaClone;
        (&&&bevy_ecs::component::DefaultCloneBehaviorSpecialization::<Self>::default())
            .default_clone_behavior()
    }

    fn on_insert() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        fn on_insert<T: Bundle + Clone>(mut world: DeferredWorld, context: HookContext) {
            let signal = world.get::<OptionSignal<T>>(context.entity).unwrap();

            let state_entity = signal.entity;
            let inserter = |any: &dyn Any, mut entity: EntityCommands| {
                let value = any.downcast_ref::<Option<T>>().unwrap();

                match value.as_ref() {
                    Some(value) => {
                        entity.reactive_cleanup::<T>().insert(value.clone());
                    }
                    None => {
                        entity.reactive_cleanup::<T>().remove_with_requires::<T>();
                    }
                }
            };

            if let Some(mut state) = world.get_mut::<SignalStateContainer>(state_entity) {
                state.insert(system::Inserter::Ptr(inserter), context.entity);
            }
        }

        Some(on_insert::<T>)
    }

    fn on_replace() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        fn on_replace<T: Clone + Bundle>(mut world: DeferredWorld, context: HookContext) {
            let state = world.get::<OptionSignal<T>>(context.entity).unwrap().entity;
            if let Some(mut state) = world.get_mut::<SignalStateContainer>(state) {
                state.remove(context.entity);
            }
        }

        Some(on_replace::<T>)
    }
}

fn gc_pass(
    signals: Query<(Entity, &SignalGc)>,
    frequency: Res<SweepFrequency>,
    mut last_sweep: Local<Option<bevy_platform::time::Instant>>,
    mut commands: Commands,
) {
    let now = bevy_platform::time::Instant::now();
    let last = match last_sweep.as_ref() {
        Some(s) => *s,
        None => {
            *last_sweep = Some(now);
            now
        }
    };

    if now.duration_since(last) >= frequency.0 {
        log::debug!("Total signals: {}", signals.iter().len());
        *last_sweep = Some(now);

        for (entity, signal) in &signals {
            if Arc::strong_count(&signal.signal) <= signal.rest_strong_count {
                commands.entity(entity).despawn();
            }
        }
    }
}

#[cfg(test)]
mod test {
    use bevy_app::prelude::*;
    use bevy_ecs::{prelude::*, system::RunSystemOnce};

    use crate::signal::SweepFrequency;

    #[derive(Component, PartialEq, Clone)]
    struct TestData(pub f32);

    #[test]
    fn basic_test() {
        use crate::prelude::*;

        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        let test_entity = world.spawn(TestData(1.0)).id();

        for _ in 0..10 {
            let mut commands = world.commands();
            let derived = commands
                .derive(move |data: SQuery<&TestData>| data.get(test_entity).unwrap().clone());
            commands.spawn(derived);
        }

        app.world_mut().get_mut::<TestData>(test_entity).unwrap().0 += 0.1;
        app.update();

        app.world_mut()
            .run_system_once(|vals: Query<&TestData>| {
                assert_eq!(11, vals.iter().len());
                assert!(vals.iter().all(|v| v.0 == 1.1))
            })
            .unwrap();
    }

    #[test]
    fn test_fanout() {
        use crate::prelude::*;

        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        let test_entity = world.spawn(TestData(1.0)).id();

        let mut commands = world.commands();
        let derived =
            commands.derive(move |data: SQuery<&TestData>| data.get(test_entity).unwrap().clone());

        for _ in 0..10 {
            commands.spawn(derived.clone());
        }

        app.world_mut().get_mut::<TestData>(test_entity).unwrap().0 += 0.1;
        app.update();

        app.world_mut()
            .run_system_once(|vals: Query<&TestData>| {
                assert_eq!(11, vals.iter().len());
                assert!(vals.iter().all(|v| v.0 == 1.1))
            })
            .unwrap();
    }

    #[test]
    fn test_gc() {
        use crate::prelude::*;

        let mut app = App::new();
        app.add_plugins(ReactPlugin)
            .insert_resource(SweepFrequency(core::time::Duration::ZERO));
        let world = app.world_mut();

        let test_entity = world.spawn(TestData(1.0)).id();

        let mut commands = world.commands();
        let derived =
            commands.derive(move |data: SQuery<&TestData>| data.get(test_entity).unwrap().clone());
        let derived = commands.spawn(derived).id();

        app.update();

        let count = app
            .world_mut()
            .run_system_once(|vals: Query<&super::SignalGc>| vals.iter().count())
            .unwrap();
        assert_eq!(1, count);

        app.world_mut().despawn(derived);

        app.update();
        let count = app
            .world_mut()
            .run_system_once(|vals: Query<&super::SignalGc>| vals.iter().count())
            .unwrap();
        assert_eq!(0, count);
    }
}
