use bevy_ecs::{prelude::*, world::CommandQueue};
use bevy_utils::prelude::DebugName;
use std::{any::Any, sync::Arc};

use crate::signal::SignalInner;

use super::SignalOutputs;

pub struct MemoSystem<S, O> {
    system: S,
    queue: CommandQueue,
    signal: Arc<SignalInner<O>>,
}

impl MemoSystem<(), ()> {
    pub fn new<S, O, M>(system: S, signal: Arc<SignalInner<O>>) -> MemoSystem<S::System, O>
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        O: Clone + PartialEq + Send + Sync + 'static,
    {
        MemoSystem {
            system: IntoSystem::into_system(system),
            queue: CommandQueue::default(),
            signal,
        }
    }
}

impl<S, O> System for MemoSystem<S, O>
where
    S: System<In = (), Out = O>,
    O: Clone + PartialEq + Send + Sync + 'static,
{
    type In = InMut<'static, SignalOutputs>;
    type Out = ();

    fn apply_deferred(&mut self, world: &mut World) {
        self.system.apply_deferred(world);
        self.queue.apply(world);
    }

    fn check_change_tick(&mut self, check: bevy_ecs::change_detection::CheckChangeTicks) {
        self.system.check_change_tick(check)
    }

    fn default_system_sets(&self) -> Vec<bevy_ecs::schedule::InternedSystemSet> {
        self.system.default_system_sets()
    }

    fn flags(&self) -> bevy_ecs::system::SystemStateFlags {
        self.system.flags()
    }

    fn get_last_run(&self) -> bevy_ecs::change_detection::Tick {
        self.system.get_last_run()
    }

    fn has_deferred(&self) -> bool {
        true
    }

    fn initialize(&mut self, world: &mut World) -> bevy_ecs::query::FilteredAccessSet {
        self.system.initialize(world)
    }

    fn is_exclusive(&self) -> bool {
        self.system.is_exclusive()
    }

    fn is_send(&self) -> bool {
        self.system.is_send()
    }

    fn name(&self) -> DebugName {
        self.system.name()
    }

    fn queue_deferred(&mut self, mut world: bevy_ecs::world::DeferredWorld) {
        self.system.queue_deferred(world.reborrow());
        world.commands().append(&mut self.queue);
    }

    unsafe fn run_unsafe(
        &mut self,
        input: SystemIn<'_, Self>,
        world: bevy_ecs::world::unsafe_world_cell::UnsafeWorldCell,
    ) -> core::result::Result<Self::Out, bevy_ecs::system::RunSystemError> {
        let value = unsafe { self.system.run_unsafe((), world) }?;

        let entities = world.entities();
        let allocator = world.entities_allocator();

        let mut current = self.signal.value.write().unwrap();
        if let Some(inner) = current.as_ref()
            && inner == &value
        {
            let any = inner as &dyn Any;
            let mut commands = Commands::new_from_entities(&mut self.queue, allocator, entities);

            input.process_new_inserters(|inserter, entity| {
                inserter.call(any, commands.entity(entity));
            });

            return Ok(());
        }

        let any = &value as &dyn Any;
        input.outputs_changed = true;

        let mut commands = Commands::new_from_entities(&mut self.queue, allocator, entities);

        input.process_all_inserters(|inserter, entity| {
            inserter.call(any, commands.entity(entity));
        });

        *current = Some(value);
        self.signal
            .tick
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }

    fn set_last_run(&mut self, last_run: bevy_ecs::change_detection::Tick) {
        self.system.set_last_run(last_run);
    }

    fn type_id(&self) -> core::any::TypeId {
        core::any::TypeId::of::<Self>()
    }

    unsafe fn validate_param_unsafe(
        &mut self,
        world: bevy_ecs::world::unsafe_world_cell::UnsafeWorldCell,
    ) -> core::result::Result<(), bevy_ecs::system::SystemParamValidationError> {
        unsafe { self.system.validate_param_unsafe(world) }
    }
}

unsafe impl<S, O> ReadOnlySystem for MemoSystem<S, O>
where
    S: System<In = (), Out = O> + ReadOnlySystem,
    O: Clone + PartialEq + Send + Sync + 'static,
{
}
