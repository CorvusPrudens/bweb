use bevy_ecs::{prelude::*, system::SystemParam};

use crate::target::{EntityTarget, Targets};

#[derive(SystemParam)]
pub struct SWorld<'w> {
    world: &'w World,
}

impl<'w> SWorld<'w> {
    pub fn get<C: Component>(&self, target: impl Into<EntityTarget>) -> Option<&'_ C> {
        let entity = target.into().get(&self.world.resource::<Targets>())?;

        let id = self.world.component_id::<C>()?;
        if let Some(observer) = super::reactive_observer::SignalObserver::get() {
            observer.add_components(entity.into(), &[id]);
        }

        self.world.get(entity)
    }
}
