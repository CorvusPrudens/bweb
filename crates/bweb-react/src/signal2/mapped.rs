use std::{any::TypeId, marker::PhantomData, sync::Mutex};

use bevy_ecs::{
    bundle::Bundle,
    lifecycle::HookContext,
    prelude::*,
    query::QueryData,
    system::{Adapt, AdapterSystem, BoxedReadOnlySystem, RunSystemError, SystemIn, SystemInput},
    world::CommandQueue,
    world::DeferredWorld,
};

use crate::signal2::{
    ReactiveWork, RegisteredSignalSystem, SharedSignalSystems, SignalCtx, SignalData,
    SubscriberClosure, SubscriberContext, SubscriberSet, TargetedSignalCtx, release_signal_system,
    source::{SourceSignal, Watching},
};

pub struct MappedSignal<D: SignalData, O> {
    pub(crate) signal: SourceSignal<D>,
    mapper: for<'w, 's> fn(D::Item<'w, 's>) -> O,
}

type Mapper<D, O> = for<'w, 's> fn(<D as QueryData>::Item<'w, 's>) -> O;

impl<D: SignalData> SourceSignal<D> {
    pub fn map<O>(&self, mapper: Mapper<D, O>) -> MappedSignal<D, O> {
        MappedSignal {
            signal: self.clone(),
            mapper,
        }
    }
}

impl<D: SignalData, O: Bundle> bevy_ecs::component::Component for MappedSignal<D, O>
where
    Self: Send + Sync + 'static,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    const STORAGE_TYPE: bevy_ecs::component::StorageType = bevy_ecs::component::StorageType::Table;

    type Mutability = bevy_ecs::component::Mutable;
    fn register_required_components(
        _requiree: bevy_ecs::component::ComponentId,
        _required_components: &mut bevy_ecs::component::RequiredComponentsRegistrator,
    ) {
    }

    fn clone_behavior() -> bevy_ecs::component::ComponentCloneBehavior {
        bevy_ecs::component::ComponentCloneBehavior::Ignore
    }

    fn relationship_accessor() -> Option<bevy_ecs::relationship::ComponentRelationshipAccessor<Self>>
    {
        None
    }

    fn on_insert() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        Some(|mut world: DeferredWorld, ctx: HookContext| {
            let entity = ctx.entity;
            let mapped = world
                .get::<MappedSignal<D, O>>(entity)
                .expect("self should be accessible");
            let signal = mapped.signal.data.entity;
            let mapper = mapped.mapper;

            // Deferred rather than done inline: `watch` is itself a queued
            // command, so `Watching` may not have landed yet, and the initial
            // evaluation below needs `&mut World` to apply what it writes.
            world.commands().queue(move |world: &mut World| {
                let Some(&Watching(source)) = world.get::<Watching>(signal) else {
                    log::warn!(
                        "signal2: mapped signal on {entity} was inserted before its source \
                         was watched; the subscription has been dropped"
                    );
                    return;
                };

                let closure: SubscriberClosure<D> = Box::new(move |data, mut ctx| {
                    ctx.commands.entity(entity).insert(mapper(data));
                });

                // A new subscriber has to be brought up to date here: the scan
                // only visits changed entities, and the source it just attached
                // to may not change again for a long time (or ever).
                let mut queue = CommandQueue::default();
                {
                    let mut work = ReactiveWork::default();
                    let commands = Commands::new(&mut queue, world);

                    if let Some(item) = world
                        .get_entity(source)
                        .ok()
                        .and_then(|entity| entity.get_components::<D>().ok())
                    {
                        closure(
                            item,
                            SubscriberContext {
                                world,
                                work: &mut work,
                                commands,
                            },
                        );
                    }
                }
                queue.apply(world);

                let mut source_entity = world.entity_mut(source);
                let mut subs = source_entity
                    .entry::<SubscriberSet<D>>()
                    .or_default()
                    .into_mut();
                subs.push(entity, closure);
            });
        })
    }

    /// Drop the subscription when the mapped signal goes away.
    ///
    /// `on_replace` is the one hook that fires for all three endings —
    /// overwritten, removed, despawned — so a re-insert also can't leave the old
    /// closure behind to write into the entity twice per change.
    fn on_replace() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        Some(|mut world: DeferredWorld, ctx: HookContext| {
            let entity = ctx.entity;
            let Some(mapped) = world.get::<MappedSignal<D, O>>(entity) else {
                return;
            };
            let signal = mapped.signal.data.entity;

            let Some(&Watching(source)) = world.get::<Watching>(signal) else {
                // Never finished subscribing, so there is nothing to undo.
                return;
            };

            world.commands().queue(move |world: &mut World| {
                let Ok(mut source_entity) = world.get_entity_mut(source) else {
                    return;
                };
                if let Some(mut subs) = source_entity.get_mut::<SubscriberSet<D>>() {
                    subs.remove_owner(entity);
                }
            });
        })
    }
}

/// A mapper that is a whole system rather than a plain function: it sees the
/// signal's data like [`MappedSignal`] does, but also gets its own
/// `SystemParam`s and a command queue.
///
/// The output bundle is inserted onto the entity holding this component, same
/// as a plain map.
pub struct MappedSignalSystem<D: SignalData, S, O> {
    pub(crate) signal: SourceSignal<D>,
    /// Taken by `on_insert`, which hands it to the registry. `None` afterwards.
    mapper: Option<S>,
    /// Whether this mapper can share one registration with every other
    /// subscriber of its type — see [`SharedSignalSystems`].
    shared: bool,
    /// Where the mapper ended up, so `on_replace` can release it without having
    /// to reach the source entity (which may already be despawned).
    registered: Option<Entity>,
    marker: PhantomData<fn() -> O>,
}

impl<D: SignalData> SourceSignal<D> {
    /// Map this signal through a full system.
    ///
    /// The system is read-only and must stay that way — it runs inside the scan,
    /// which holds nothing but `&World`. Write through the `commands` on its
    /// [`SignalCtx`] rather than through a `Commands` param: a `Commands` param
    /// gets a private queue that nothing in the scan can flush.
    pub fn map_system<S, O, M>(&self, system: S) -> MappedSignalSystem<D, S::System, O>
    where
        S: IntoSystem<SignalCtx<'static, D>, O, M>,
        S::System: ReadOnlySystem,
    {
        // Measured on the value handed in, not on the system built from it: a
        // `fn` item or a non-capturing closure is zero-sized, and two of them
        // with the same type cannot behave differently, which is exactly the
        // condition for sharing one instance.
        let shared = size_of::<S>() == 0;
        let mapper = IntoSystem::into_system(system);
        MappedSignalSystem {
            signal: self.clone(),
            mapper: Some(mapper),
            shared,
            registered: None,
            marker: PhantomData,
        }
    }
}

/// Erases a mapper's output type down to `()` by doing the insert itself.
///
/// The scan is generic over `D` alone, so it can neither name `O` nor know
/// which entity a given run's output belongs to. Both facts are handled here:
/// the target arrives through the input, and what comes back out is a system
/// uniform enough for the scan to hold in one box.
pub(crate) struct InsertOutput<D, O>(PhantomData<fn() -> (D, O)>);

impl<D, O, S> Adapt<S> for InsertOutput<D, O>
where
    D: SignalData,
    O: Bundle,
    S: ReadOnlySystem<In = SignalCtx<'static, D>, Out = O>,
{
    type In = TargetedSignalCtx<'static, D>;
    type Out = ();

    fn adapt(
        &mut self,
        input: <Self::In as SystemInput>::Inner<'_>,
        run_system: impl FnOnce(SystemIn<'_, S>) -> Result<S::Out, RunSystemError>,
    ) -> Result<Self::Out, RunSystemError> {
        let (target, data, mut commands) = input;
        // Anything the mapper queued itself lands ahead of the insert, on the
        // same queue: one ordering, and the scan's own flush covers both.
        let output = run_system((D::shrink(data), commands.reborrow()))?;
        commands.entity(target).insert(output);
        Ok(())
    }
}

/// Find or create the entity holding `mapper`, claiming one use of it.
fn register_mapper<D, S, O>(world: &mut World, mapper: S, shared: bool) -> Entity
where
    D: SignalData,
    O: Bundle,
    S: ReadOnlySystem<In = SignalCtx<'static, D>, Out = O>,
{
    let id = TypeId::of::<S>();

    if shared
        && let Some(&existing) = world.resource::<SharedSignalSystems>().0.get(&id)
        && let Some(mut registered) = world.get_mut::<RegisteredSignalSystem<D>>(existing)
    {
        registered.users += 1;
        return existing;
    }

    let name = mapper.name();
    let mut system: BoxedReadOnlySystem<TargetedSignalCtx<'static, D>> = Box::new(
        AdapterSystem::new(InsertOutput::<D, O>(PhantomData), mapper, name),
    );
    system.initialize(world);

    if system.has_deferred() {
        log::error!(
            "signal2: mapper system `{}` has a deferred parameter (a `Commands` or `Deferred` \
             param). The scan runs it with `&World` and can never flush that queue, so anything \
             written through it will be silently dropped — take the `commands` on the mapper's \
             `SignalCtx` instead.",
            system.name()
        );
    }

    let system = world
        .spawn(RegisteredSignalSystem::<D> {
            id,
            users: 1,
            system: Mutex::new(system),
        })
        .id();

    if shared {
        world
            .resource_mut::<SharedSignalSystems>()
            .0
            .insert(id, system);
    }

    system
}

/// Run a registered mapper once, outside the scan.
///
/// This is the same debt [`MappedSignal`] settles inline when it attaches: the
/// scan only visits entities that changed, and a source that a subscriber just
/// attached to may not change again for a long time.
fn run_registered_mapper<D: SignalData>(
    world: &mut World,
    system: Entity,
    target: Entity,
    source: Entity,
) {
    let mut queue = CommandQueue::default();
    {
        let Some(registered) = world.get::<RegisteredSignalSystem<D>>(system) else {
            return;
        };
        let Ok(mut registered) = registered.system.lock() else {
            log::error!("signal2: mapper system {system} panicked on an earlier run");
            return;
        };
        let Some(data) = world
            .get_entity(source)
            .ok()
            .and_then(|entity| entity.get_components::<D>().ok())
        else {
            return;
        };

        // A private queue rather than the world's: `run_readonly` needs `&World`
        // for the duration, and applying is what wants `&mut`.
        let commands = Commands::new(&mut queue, world);
        if let Err(e) = registered.run_readonly((target, D::shrink(data), commands), world) {
            log::error!("signal2: mapper system failed its first run: {e}");
        }
    }
    queue.apply(world);
}

impl<D: SignalData, S, O: Bundle> bevy_ecs::component::Component for MappedSignalSystem<D, S, O>
where
    Self: Send + Sync + 'static,
    for<'w, 's> D::Item<'w, 's>: Copy,
    S: ReadOnlySystem<In = SignalCtx<'static, D>, Out = O>,
{
    const STORAGE_TYPE: bevy_ecs::component::StorageType = bevy_ecs::component::StorageType::Table;

    type Mutability = bevy_ecs::component::Mutable;
    fn register_required_components(
        _requiree: bevy_ecs::component::ComponentId,
        _required_components: &mut bevy_ecs::component::RequiredComponentsRegistrator,
    ) {
    }

    fn clone_behavior() -> bevy_ecs::component::ComponentCloneBehavior {
        bevy_ecs::component::ComponentCloneBehavior::Ignore
    }

    fn relationship_accessor() -> Option<bevy_ecs::relationship::ComponentRelationshipAccessor<Self>>
    {
        None
    }

    fn on_insert() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        Some(|mut world: DeferredWorld, ctx: HookContext| {
            let entity = ctx.entity;
            let Some(mut mapped) = world.get_mut::<MappedSignalSystem<D, S, O>>(entity) else {
                return;
            };
            let signal = mapped.signal.data.entity;
            let shared = mapped.shared;
            let Some(mapper) = mapped.mapper.take() else {
                log::warn!(
                    "signal2: mapped signal system on {entity} was inserted twice; the second \
                     insert has been dropped"
                );
                return;
            };

            // Deferred rather than done inline: `watch` is itself a queued
            // command, so `Watching` may not have landed yet, and registering
            // the mapper needs `&mut World` to initialize it.
            world.commands().queue(move |world: &mut World| {
                // The owner can be despawned between the insert and this
                // command, in which case its `on_replace` has already run and
                // has no registration to release — so don't make one.
                if world.get_entity(entity).is_err() {
                    return;
                }

                let Some(&Watching(source)) = world.get::<Watching>(signal) else {
                    log::warn!(
                        "signal2: mapped signal on {entity} was inserted before its source \
                         was watched; the subscription has been dropped"
                    );
                    return;
                };

                let system = register_mapper::<D, S, O>(world, mapper, shared);
                if let Some(mut mapped) = world.get_mut::<MappedSignalSystem<D, S, O>>(entity) {
                    mapped.registered = Some(system);
                }

                run_registered_mapper::<D>(world, system, entity, source);

                let mut source_entity = world.entity_mut(source);
                let mut subs = source_entity
                    .entry::<SubscriberSet<D>>()
                    .or_default()
                    .into_mut();
                subs.push_system(entity, system);
            });
        })
    }

    /// Drop the subscription when the mapped signal goes away.
    ///
    /// `on_replace` is the one hook that fires for all three endings —
    /// overwritten, removed, despawned — so a re-insert also can't leave the old
    /// subscription behind to write into the entity twice per change.
    fn on_replace() -> Option<bevy_ecs::lifecycle::ComponentHook> {
        Some(|mut world: DeferredWorld, ctx: HookContext| {
            let entity = ctx.entity;
            let Some(mapped) = world.get::<MappedSignalSystem<D, S, O>>(entity) else {
                return;
            };
            let signal = mapped.signal.data.entity;
            let registered = mapped.registered;
            let source = world
                .get::<Watching>(signal)
                .map(|&Watching(source)| source);

            world.commands().queue(move |world: &mut World| {
                if let Some(source) = source
                    && let Ok(mut source_entity) = world.get_entity_mut(source)
                    && let Some(mut subs) = source_entity.get_mut::<SubscriberSet<D>>()
                {
                    subs.remove_owner(entity);
                }

                // Released off the component rather than off `remove_owner`'s
                // return, so a subscriber outliving its source still gives its
                // registration back instead of leaking it.
                if let Some(registered) = registered {
                    release_signal_system::<D>(world, registered);
                }
            });
        })
    }
}
