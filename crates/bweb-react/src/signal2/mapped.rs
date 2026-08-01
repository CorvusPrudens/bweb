use bevy_ecs::{bundle::Bundle, lifecycle::HookContext, query::QueryData, world::DeferredWorld};

use crate::signal2::{
    SignalData, SubscriberSet,
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
            let watching = mapped.signal.data.entity;
            let mapper = mapped.mapper;

            if let Some(&watching) = world.get::<Watching>(watching) {
                match world.get_mut::<SubscriberSet<D>>(watching.0) {
                    Some(mut subs) => {
                        subs.closures.push(Box::new(move |data, mut commands| {
                            let result = mapper(data);
                            commands.entity(entity).insert(result);
                        }));
                    }
                    None => {
                        world
                            .commands()
                            .entity(watching.0)
                            .entry::<SubscriberSet<D>>()
                            .or_default()
                            .and_modify(move |mut subs| {
                                subs.closures.push(Box::new(move |data, mut commands| {
                                    let result = mapper(data);
                                    commands.entity(entity).insert(result);
                                }));
                            });
                    }
                }
            }
        })
    }
}
