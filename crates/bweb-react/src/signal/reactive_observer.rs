use bevy_ecs::{
    change_detection::Tick,
    component::ComponentId,
    prelude::*,
};
use bevy_platform::collections::HashSet;
use std::sync::{Arc, RwLock, Weak};
use std::{cell::RefCell, collections::HashMap};

use crate::signal::SignalTick;

#[derive(Component, Clone)]
pub struct SubscriberSet(Arc<RwLock<SignalSetInner>>);

impl SubscriberSet {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(SignalSetInner::default())))
    }

    pub fn add_signal(&self, signal: SignalSubscriber) {
        self.0.write().unwrap().signals.push(signal)
    }

    pub fn add_components(&self, entity: Entity, components: &[ComponentId]) {
        let mut writer = self.0.write().unwrap();

        writer.entities.push(EntitySubscriber {
            entity,
            components: components.to_vec(),
        });
    }

    pub fn add_resource(&self, components: ComponentId) {
        let mut writer = self.0.write().unwrap();
        writer.resources.push(components);
    }

    pub fn clear(&self) {
        let mut inner = self.0.write().unwrap();

        inner.entities.clear();
        inner.resources.clear();
        inner.signals.clear();
    }

    pub fn has_changed(&self, world: &World, last_run: Tick, this_run: Tick) -> bool {
        let inner = self.0.read().unwrap();
        let removed = world.resource::<RemovedSet>();

        for entity_set in &inner.entities {
            if let Ok(entity) = world.get_entity(entity_set.entity) {
                if entity_set.components.iter().any(|c| {
                    entity
                        .get_change_ticks_by_id(*c)
                        .is_some_and(|tick| tick.is_changed(last_run, this_run))
                        || removed
                            .0
                            .get(&entity_set.entity)
                            .is_some_and(|set| set.contains(c))
                }) {
                    return true;
                }
            }
        }

        for signal in &inner.signals {
            if let Some(any) = Weak::upgrade(&signal.signal)
                && any.tick() != signal.last_read
            {
                return true;
            }
        }

        for resource in &inner.resources {
            if let Some(ticks) = world.get_resource_change_ticks_by_id(*resource)
                && ticks.is_changed(last_run, this_run)
            {
                return true;
            }
        }

        false
    }
}

#[derive(Default, Resource)]
pub(crate) struct RemovedSet(HashMap<Entity, HashSet<ComponentId>>);

impl RemovedSet {
    pub fn update(world: &mut World) {
        world.resource_scope::<RemovedSet, _>(|world, mut set| {
            let removed_set = world.removed_components();

            set.0.clear();
            for (component, entities) in removed_set.iter() {
                for entity in entities.iter_current_update_messages() {
                    set.0
                        .entry(entity.clone().into())
                        .or_default()
                        .insert(*component);
                }
            }
        });

        // TODO: might cause other problems
        world.clear_trackers();
    }
}

#[derive(Default)]
pub struct SignalSetInner {
    entities: Vec<EntitySubscriber>,
    resources: Vec<ComponentId>,
    signals: Vec<SignalSubscriber>,
}

pub struct EntitySubscriber {
    entity: Entity,
    components: Vec<ComponentId>,
}

pub struct SignalSubscriber {
    signal: Weak<dyn SignalTick + Send + Sync>,
    last_read: u32,
}

impl SignalSubscriber {
    pub fn new<T: SignalTick + Send + Sync + 'static>(signal: &Arc<T>) -> Self {
        let last_read = signal.tick();
        Self {
            signal: Arc::downgrade(signal) as Weak<dyn SignalTick + Send + Sync>,
            last_read,
        }
    }
}

thread_local! {
    static OBSERVER: RefCell<Option<SubscriberSet>> = const { RefCell::new(None) };
}

/// The current reactive observer.
///
/// The observer is whatever reactive node is currently listening for signals that need to be
/// tracked. For example, if an effect is running, that effect is the observer, which means it will
/// subscribe to changes in any signals that are read.
pub struct SignalObserver;

impl SignalObserver {
    pub fn observe<F, O>(subscriber: &SubscriberSet, f: F) -> O
    where
        F: FnOnce() -> O,
    {
        Self::set(Some(subscriber.clone()));
        let result = f();
        Self::take();

        result
    }

    /// Returns the current observer, if any.
    pub fn get() -> Option<SubscriberSet> {
        OBSERVER.with_borrow(|obs| obs.as_ref().cloned())
    }

    fn take() -> Option<SubscriberSet> {
        OBSERVER.with_borrow_mut(Option::take)
    }

    fn set(observer: Option<SubscriberSet>) {
        OBSERVER.with_borrow_mut(|o| *o = observer);
    }
}
