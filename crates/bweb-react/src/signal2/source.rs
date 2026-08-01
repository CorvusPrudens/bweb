use std::{marker::PhantomData, sync::Arc};

use bevy_ecs::{prelude::*, query::ReleaseStateQueryData, system::SystemParam};

use crate::signal2::{ReactiveSystems, SignalData, SubscriberSet};

pub struct SourceSignal<D> {
    pub(crate) data: Arc<InnerData>,
    marker: PhantomData<fn() -> D>,
}

impl<D> Clone for SourceSignal<D> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            marker: PhantomData,
        }
    }
}

pub(crate) struct InnerData {
    pub(crate) entity: Entity,
}

#[derive(Component, Clone, Copy)]
pub struct Watching(pub Entity);

pub struct SourceSignalView<'a, 'w, 's, D>(
    pub(crate) SourceSignal<D>,
    pub(crate) &'a mut Commands<'w, 's>,
);

impl<'a, 'w, 's, D: SignalData> SourceSignalView<'a, 'w, 's, D>
where
    for<'x, 'y> D::Item<'x, 'y>: Copy,
{
    pub fn watch(self, entity: Entity) -> SourceSignal<D> {
        let signal_entity = self.0.data.entity;
        self.1.queue(move |world: &mut World| {
            world.entity_mut(signal_entity).insert(Watching(entity));
            world
                .entity_mut(entity)
                .entry::<SubscriberSet<D>>()
                .or_default();
        });
        self.0
    }
}

impl<D> SourceSignal<D>
where
    D: SignalData,
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    pub fn from_commands(commands: &mut Commands) -> Self {
        let source_entity = commands.spawn_empty().id();
        let data = Arc::new(InnerData {
            entity: source_entity,
        });
        commands.queue(|world: &mut World| {
            world.resource_mut::<ReactiveSystems>().register::<D>();
        });

        SourceSignal {
            data,
            marker: PhantomData,
        }
    }
}

#[derive(SystemParam)]
pub struct SignalStore<'w, 's> {
    values: Query<'w, 's, EntityRef<'static>>,
}

impl<D: SignalData + ReleaseStateQueryData> SourceSignal<D> {
    pub fn get<'a>(&self, store: &'a SignalStore<'_, '_>) -> Result<D::Item<'a, 'static>> {
        let entity = store.values.get(self.data.entity)?;
        let item = entity.get_components::<D>()?;
        Ok(item)
    }
}
