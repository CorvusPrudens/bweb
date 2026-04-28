use bevy_ecs::{prelude::*, world::CommandQueue};
use bevy_utils::prelude::DebugName;
use std::{any::Any, sync::Arc};

use crate::signal::SignalInner;

use super::SignalOutputs;

pub struct DerivedSystem<S, O> {
    system: S,
    queue: CommandQueue,
    signal: Arc<SignalInner<O>>,
}

impl DerivedSystem<(), ()> {
    pub fn new<S, O, M>(system: S, signal: Arc<SignalInner<O>>) -> DerivedSystem<S::System, O>
    where
        S: IntoSystem<(), O, M> + Send + Sync + 'static,
        O: Clone + Send + Sync + 'static,
    {
        DerivedSystem {
            system: IntoSystem::into_system(system),
            queue: CommandQueue::default(),
            signal,
        }
    }
}

impl<S, O> System for DerivedSystem<S, O>
where
    S: System<In = (), Out = O>,
    O: Clone + Send + Sync + 'static,
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
        let entities = world.entities();
        let allocator = world.entities_allocator();

        if !input.inputs_changed {
            if !input.new_inserters.is_empty() {
                let value = unsafe { self.system.run_unsafe((), world) }?;
                let any = &value as &dyn Any;
                let mut commands = Commands::new_from_entities(&mut self.queue, allocator, entities);

                input.process_new_inserters(|inserter, entity| {
                    inserter.call(any, commands.entity(entity));
                });
            }

            return Ok(());
        }

        let value = unsafe { self.system.run_unsafe((), world) }?;
        if input.inserters.is_empty() && input.new_inserters.is_empty() {
            self.signal.set_inner(value);
            return Ok(());
        }

        self.signal.set_inner(value.clone());

        let any = &value as &dyn Any;
        let mut commands = Commands::new_from_entities(&mut self.queue, allocator, entities);

        input.process_all_inserters(|inserter, entity| {
            inserter.call(any, commands.entity(entity));
        });

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

unsafe impl<S, O> ReadOnlySystem for DerivedSystem<S, O>
where
    S: System<In = (), Out = O> + ReadOnlySystem,
    O: Clone + Send + Sync + 'static,
{
}
