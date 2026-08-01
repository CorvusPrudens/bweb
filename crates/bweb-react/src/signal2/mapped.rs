use bevy_ecs::{
    bundle::Bundle, lifecycle::HookContext, prelude::*, query::QueryData, world::CommandQueue,
    world::DeferredWorld,
};

use crate::signal2::{
    ReactiveWork, SignalData, SubscriberClosure, SubscriberContext, SubscriberSet,
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
