use super::{DomSystems, html::EventTarget};
use crate::{ScheduleTrigger, js_err::JsErr};
use bevy_app::prelude::*;
use bevy_ecs::{
    error::ErrorContext,
    lifecycle::HookContext,
    prelude::*,
    system::{RegisteredSystemError, SystemId},
    world::DeferredWorld,
};
use bevy_query_observer::{AddStopObserver, Stop};
use bevy_utils::prelude::DebugName;
use send_wrapper::SendWrapper;
use std::{any::TypeId, collections::HashSet};
use wasm_bindgen::{JsCast, convert::FromWasmAbi, prelude::Closure};

mod defer;
mod handler;

pub use defer::*;
pub use handler::*;

pub(super) struct EventsPlugin;

impl Plugin for EventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Bevent::plugin);
    }
}

#[derive(Component)]
pub struct Bevent {
    handler:
        Option<Box<dyn FnOnce(&mut World) -> (Handler<web_sys::Event>, DebugName) + Send + Sync>>,
    event: &'static str,
    trigger: bool,
    capturing: bool,
}

impl Bevent {
    pub fn new<E, S, M>(event: &'static str, system: S) -> Self
    where
        S: IntoHandlerSystem<E, M> + Send + Sync + 'static,
        E: JsCast + 'static,
    {
        let adapter = |e: Ev<web_sys::Event>| -> Result<_> {
            Ok(JsEvent {
                entity: e.entity,
                event: SendWrapper::new(
                    e.0.event
                        .take()
                        .dyn_into::<E>()
                        .map_err(|_| "Failed to convert JS event to target")?,
                ),
            })
        };
        let adapter = IntoSystem::<_, JsEvent<E>, _>::into_system(adapter);
        let compound = adapter.pipe(system.into_handler());

        Self {
            handler: Some(Box::new(move |world: &mut World| {
                let name = DebugName::type_name::<S>();
                (world.register_system(compound), name)
            })),
            event,
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

    fn transform(world: &mut World) {
        let mut clicks =
            world.query_filtered::<Entity, (With<Self>, Without<EventHandler<web_sys::Event>>)>();
        let clicks: Vec<_> = clicks.iter(world).collect();

        for click in clicks {
            let mut ev = world.entity_mut(click);
            let mut ev = ev.get_mut::<Self>().unwrap();

            let handler = ev.handler.take().unwrap();
            let trigger = ev.trigger;
            let capturing = ev.capturing;
            let event = ev.event;

            let (id, name) = handler(world);
            world.entity_mut(click).insert(EventHandler {
                handler: id,
                event,
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
            .insert(core::any::TypeId::of::<web_sys::Event>())
        {
            app.add_systems(
                PostUpdate,
                manage_handlers::<web_sys::Event>.after(DomSystems::Attach),
            )
            .add_stop_observer(EventHandler::<web_sys::Event>::stop_event);
        }
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

pub type Ev<E> = In<JsEvent<E>>;

#[derive(Clone)]
pub struct JsEvent<E> {
    entity: Entity,
    event: SendWrapper<E>,
}

impl<E> JsEvent<E> {
    pub fn target(&self) -> Entity {
        self.entity
    }
}

impl<E> AsRef<E> for JsEvent<E> {
    fn as_ref(&self) -> &E {
        &self.event
    }
}

impl<E> core::ops::Deref for JsEvent<E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

#[derive(Resource, Default)]
struct RegisteredEvents(HashSet<TypeId>);

type Handler<E> = SystemId<In<JsEvent<E>>, Result>;

macro_rules! handler {
    ($func:ident, $name:literal, $ev:path) => {
        pub fn $func<S, M>(system: S) -> super::Bevent
        where
            S: super::IntoHandlerSystem<$ev, M> + Send + Sync + 'static,
        {
            super::Bevent::new($name, system)
        }
    };
}

pub mod ev {
    handler! { click, "click", web_sys::PointerEvent }
    handler! { pointer_down, "pointerdown", web_sys::PointerEvent }
    handler! { pointer_move, "pointermove", web_sys::PointerEvent }
    handler! { pointer_up, "pointerup", web_sys::PointerEvent }
    handler! { pointer_enter, "pointerenter", web_sys::PointerEvent }
    handler! { pointer_leave, "pointerleave", web_sys::PointerEvent }
    handler! { context_menu, "contextmenu", web_sys::PointerEvent }
    handler! { pop_state, "popstate", web_sys::PopStateEvent }
    handler! { select_start, "selectstart", web_sys::Event }
    handler! { key_down, "keydown", web_sys::KeyboardEvent }
    handler! { blur, "blur", web_sys::FocusEvent }
    handler! { input, "input", web_sys::InputEvent }
    handler! { change, "change", web_sys::Event }
    handler! { wheel, "wheel", web_sys::WheelEvent }
}

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
    fn stop_event(data: Stop<(&EventOf, &Self)>, target: Query<&EventTarget>) -> Result {
        let (target_entity, handler) = data.into_inner();

        if let Ok(target) = target.get(target_entity.0)
            && let Some(closure) = handler.closure.as_ref()
        {
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

    // This runs after the above observer when the handler is removed.
    fn on_replace_hook(mut world: DeferredWorld, context: HookContext) {
        let Some(handler) = world.get::<Self>(context.entity) else {
            return;
        };
        let handler_system = handler.handler;
        world.commands().unregister_system(handler_system);
    }
}

fn manage_handlers<E>(
    mut handlers: Query<
        (Entity, &mut EventHandler<E>, &EventOf),
        Or<(Changed<EventHandler<E>>, Changed<EventOf>)>,
    >,
    nodes: Query<&EventTarget>,
) -> Result
where
    E: FromWasmAbi + 'static,
{
    for (entity, mut handler, node_entity) in &mut handlers {
        let node = nodes.get(node_entity.0)?;

        match handler.closure.as_ref() {
            Some(closure) => {
                node.add_event_listener_with_callback_and_bool(
                    handler.event,
                    closure.as_ref().unchecked_ref(),
                    handler.capturing,
                )
                .js_err()?;
            }
            None => {
                let event_name = handler.event;
                let id = handler.handler;
                let trigger = handler.trigger;
                let name = handler.name.clone();
                let function = Closure::new(move |ev: E| {
                    let res = crate::web_runner::app_scope(|app| {
                        let world = app.world_mut();

                        if trigger {
                            match event_name {
                                "pointerdown" | "mousedown" | "keydown" => {
                                    // prefer synchronous execution for paired events
                                    world.resource_mut::<ScheduleTrigger>().trigger();
                                }
                                _ => {
                                    world.resource_mut::<ScheduleTrigger>().trigger_async();
                                }
                            }
                        }

                        let result = world.run_system_with(
                            id,
                            JsEvent {
                                entity,
                                event: SendWrapper::new(ev),
                            },
                        );

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

                    if res.is_err() {
                        log::error!("Failed to borrow app for event handler");
                    }
                });

                node.add_event_listener_with_callback_and_bool(
                    handler.event,
                    function.as_ref().unchecked_ref(),
                    handler.capturing,
                )
                .js_err()?;

                handler.closure = Some(SendWrapper::new(function));
            }
        }
    }

    Ok(())
}
