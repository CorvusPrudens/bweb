use super::{DomSystems, html::EventTarget};
use crate::{js_err::JsErr, web_runner::ScheduleTrigger};
use bevy_app::prelude::*;
use bevy_ecs::{
    error::ErrorContext,
    lifecycle::HookContext,
    prelude::*,
    system::{RegisteredSystemError, SystemId},
    world::DeferredWorld,
};
use bevy_utils::prelude::DebugName;
use send_wrapper::SendWrapper;
use std::{any::TypeId, collections::HashSet};
use wasm_bindgen::{JsCast, convert::FromWasmAbi, prelude::Closure};

mod handler;

pub use handler::*;

pub(super) struct EventsPlugin;

impl Plugin for EventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            OnClick::plugin,
            OnPopState::plugin,
            OnSelectStart::plugin,
            OnPointerDown::plugin,
            OnPointerMove::plugin,
            OnPointerUp::plugin,
            OnContextMenu::plugin,
            OnKeyDown::plugin,
        ));
    }
}

#[derive(Debug, Component)]
#[relationship(relationship_target = Events)]
pub struct EventOf(pub Entity);

#[derive(Debug, Component)]
#[relationship_target(relationship = EventOf, linked_spawn)]
pub struct Events(Vec<Entity>);

#[doc(hidden)]
pub use bevy_ecs::spawn::Spawn;

#[macro_export]
macro_rules! events {
    [$($effect:expr),*$(,)?] => {
        <$crate::dom::events::Events>::spawn(($($crate::dom::events::Spawn($effect)),*))
    };
}

pub type Ev<E> = In<Event<E>>;

#[derive(Clone)]
pub struct Event<E> {
    entity: Entity,
    event: SendWrapper<E>,
}

impl<E> Event<E> {
    pub fn target(&self) -> Entity {
        self.entity
    }
}

impl<E> AsRef<E> for Event<E> {
    fn as_ref(&self) -> &E {
        &self.event
    }
}

impl<E> core::ops::Deref for Event<E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

#[derive(Resource, Default)]
struct RegisteredEvents(HashSet<TypeId>);

type Handler<E> = SystemId<In<Event<E>>, Result>;

macro_rules! handler {
    ($ty:ident, $name:literal, $ev:path) => {
        #[derive(Component)]
        pub struct $ty {
            handler: Option<Box<dyn FnOnce(&mut World) -> (Handler<$ev>, DebugName) + Send + Sync>>,
            trigger: bool,
            capturing: bool,
        }

        impl $ty {
            pub fn new<S, M>(system: S) -> Self
            where
                S: IntoHandlerSystem<$ev, M> + Send + Sync + 'static,
            {
                Self {
                    handler: Some(Box::new(move |world: &mut World| {
                        let name = DebugName::type_name::<S>();
                        (world.register_system(system.into_handler()), name)
                    })),
                    trigger: true,
                    capturing: false,
                }
            }

            /// Prevent this callback from triggering an ECS update.
            #[inline(always)]
            pub fn suppress(self) -> Self {
                Self {
                    trigger: false,
                    ..self
                }
            }

            /// Handle this event in the capturing phase.
            #[inline(always)]
            pub fn capturing(self) -> Self {
                Self {
                    capturing: true,
                    ..self
                }
            }

            pub fn stop_propagation() -> Self {
                Self::new(|e: Ev<$ev>| e.stop_propagation()).suppress()
            }

            pub fn prevent_default() -> Self {
                Self::new(|e: Ev<$ev>| e.prevent_default()).suppress()
            }

            fn transform(world: &mut World) {
                let mut clicks =
                    world.query_filtered::<Entity, (With<$ty>, Without<EventHandler<$ev>>)>();
                let clicks: Vec<_> = clicks.iter(world).collect();

                for click in clicks {
                    let mut ev = world.entity_mut(click);
                    let mut ev = ev.get_mut::<$ty>().unwrap();

                    let handler = ev.handler.take().unwrap();
                    let trigger = ev.trigger;
                    let capturing = ev.capturing;

                    let (id, name) = handler(world);
                    world.entity_mut(click).insert(EventHandler {
                        handler: id,
                        event: $name,
                        name,
                        closure: None,
                        trigger,
                        capturing,
                    });
                }
            }

            fn plugin(app: &mut App) {
                app.add_systems(PostUpdate, Self::transform.in_set(DomSystems::Attach));

                if app
                    .world_mut()
                    .get_resource_or_init::<RegisteredEvents>()
                    .0
                    .insert(core::any::TypeId::of::<$ev>())
                {
                    app.add_systems(PostUpdate, manage_handlers::<$ev>.after(DomSystems::Attach))
                        .add_observer(EventHandler::<$ev>::observe_replace_event_of);
                }
            }
        }
    };
}

handler! { OnClick, "click", web_sys::PointerEvent }
handler! { OnPointerDown, "pointerdown", web_sys::PointerEvent }
handler! { OnPointerMove, "pointermove", web_sys::PointerEvent }
handler! { OnPointerUp, "pointerup", web_sys::PointerEvent }
handler! { OnContextMenu, "contextmenu", web_sys::PointerEvent }
handler! { OnPopState, "popstate", web_sys::PopStateEvent }
handler! { OnSelectStart, "selectstart", web_sys::Event }
handler! { OnKeyDown, "keydown", web_sys::KeyboardEvent }

#[derive(Debug, Component)]
#[component(on_replace = Self::on_replace_hook)]
pub struct EventHandler<E: FromWasmAbi + 'static> {
    handler: Handler<E>,
    name: DebugName,
    event: &'static str,
    closure: Option<SendWrapper<Closure<dyn FnMut(E)>>>,
    /// Trigger an ECS update cycle.
    trigger: bool,
    capturing: bool,
}

impl<E: FromWasmAbi + 'static> EventHandler<E> {
    fn observe_replace_event_of(
        trigger: On<Replace, EventOf>,
        mut event: Query<(&EventOf, &mut Self)>,
        target: Query<&EventTarget>,
    ) -> Result {
        let Ok((target_entity, mut handler)) = event.get_mut(trigger.target()) else {
            return Ok(());
        };
        let Ok(target) = target.get(target_entity.0) else {
            return Ok(());
        };

        if let Some(closure) = handler.closure.take() {
            target
                .remove_event_listener_with_callback_and_bool(
                    handler.event,
                    closure.as_ref().unchecked_ref(),
                    handler.capturing,
                )
                .js_err()?;
        }

        Ok(())
    }

    fn on_replace_hook(mut world: DeferredWorld, context: HookContext) {
        let Some(handler) = world.get::<Self>(context.entity) else {
            return;
        };
        let handler_system = handler.handler;
        world.commands().unregister_system(handler_system);

        let handler = world.get::<Self>(context.entity).unwrap();
        let Some(event_target) = world.get::<EventOf>(context.entity).map(|e| e.0) else {
            return;
        };
        let Some(node) = world.get::<EventTarget>(event_target).cloned() else {
            return;
        };

        let mut handler = world.get_mut::<EventHandler<E>>(context.entity).unwrap();
        if let Some(closure) = handler.closure.take() {
            node.remove_event_listener_with_callback_and_bool(
                handler.event,
                closure.as_ref().unchecked_ref(),
                handler.capturing,
            )
            .unwrap();
        }
    }
}

fn manage_handlers<E>(
    mut handlers: Query<
        (Entity, &mut EventHandler<E>, &EventOf),
        (Changed<EventHandler<E>>, Changed<EventOf>),
    >,
    nodes: Query<&EventTarget>,
) -> Result
where
    E: FromWasmAbi + 'static,
{
    for (entity, mut handler, node_entity) in &mut handlers {
        let node = nodes.get(node_entity.0)?;
        let id = handler.handler;
        let trigger = handler.trigger;
        let name = handler.name.clone();
        let function = Closure::new(move |ev: E| {
            crate::web_runner::app_scope(|app| {
                let world = app.world_mut();
                let result = world.run_system_with(
                    id,
                    Event {
                        entity,
                        event: SendWrapper::new(ev),
                    },
                );

                if trigger {
                    world.resource::<ScheduleTrigger>().trigger();
                }

                world.flush();

                match result {
                    Ok(Err(e)) | Err(RegisteredSystemError::Failed(e)) => {
                        let tick = world.change_tick();
                        match app.get_error_handler() {
                            Some(error_handler) => error_handler(
                                e,
                                ErrorContext::System {
                                    name: name.clone(),
                                    last_run: tick,
                                },
                            ),
                            None => {
                                log::error!("Failed to execute event handler: {e:?}");
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to execute event handler: {e:?}");
                    }
                    Ok(Ok(())) => {}
                }
            });
        });

        node.add_event_listener_with_callback_and_bool(
            handler.event,
            function.as_ref().unchecked_ref(),
            handler.capturing,
        )
        .js_err()?;

        handler.closure = Some(SendWrapper::new(function));
    }

    Ok(())
}
