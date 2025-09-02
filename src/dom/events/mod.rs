use super::{DomSystems, html::EventTarget};
use crate::{js_err::JsErr, web_runner::ScheduleTrigger};
use bevy_app::prelude::*;
use bevy_ecs::{component::HookContext, prelude::*, system::SystemId, world::DeferredWorld};
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
        pub struct $ty(Option<Box<dyn FnOnce(&mut World) -> Handler<$ev> + Send + Sync>>);

        impl $ty {
            pub fn new<S, M>(system: S) -> Self
            where
                S: IntoHandlerSystem<$ev, M> + Send + Sync + 'static,
            {
                Self(Some(Box::new(move |world: &mut World| {
                    world.register_system(system.into_handler())
                })))
            }

            pub fn stop_propagation() -> Self {
                Self::new(|e: Ev<$ev>| e.stop_propagation())
            }

            pub fn prevent_default() -> Self {
                Self::new(|e: Ev<$ev>| e.prevent_default())
            }

            fn transform(world: &mut World) {
                let mut clicks =
                    world.query_filtered::<Entity, (With<$ty>, Without<EventHandler<$ev>>)>();
                let clicks: Vec<_> = clicks.iter(world).collect();

                for click in clicks {
                    let handler = world
                        .entity_mut(click)
                        .get_mut::<$ty>()
                        .unwrap()
                        .0
                        .take()
                        .unwrap();
                    let id = handler(world);
                    world.entity_mut(click).insert(EventHandler {
                        handler: id,
                        event: $name,
                        closure: None,
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
                    app.add_systems(PostUpdate, manage_handlers::<$ev>.after(DomSystems::Attach));
                }
            }
        }
    };
}

handler! { OnClick, "click", web_sys::PointerEvent }
handler! { OnPointerDown, "pointerdown", web_sys::PointerEvent }
handler! { OnPointerMove, "pointermove", web_sys::PointerEvent }
handler! { OnPointerUp, "pointerup", web_sys::PointerEvent }
handler! { OnPopState, "popstate", web_sys::PopStateEvent }
handler! { OnSelectStart, "selectstart", web_sys::Event }
handler! { OnKeyDown, "keydown", web_sys::KeyboardEvent }

#[derive(Debug, Component)]
#[component(on_replace = Self::on_replace_hook)]
pub struct EventHandler<E: FromWasmAbi + 'static> {
    handler: Handler<E>,
    event: &'static str,
    closure: Option<SendWrapper<Closure<dyn FnMut(E)>>>,
}

impl<E: FromWasmAbi + 'static> EventHandler<E> {
    fn on_replace_hook(mut world: DeferredWorld, context: HookContext) {
        let Some(handler) = world.get::<EventHandler<E>>(context.entity) else {
            return;
        };

        let Some(node) = world.get::<EventOf>(context.entity) else {
            return;
        };

        let Some(node) = world.get::<EventTarget>(node.0) else {
            return;
        };

        if let Some(closure) = handler.closure.as_ref() {
            node.remove_event_listener_with_callback(
                handler.event,
                closure.as_ref().unchecked_ref(),
            )
            .unwrap();
        }

        let handler = handler.handler;
        world
            .commands()
            .queue(move |world: &mut World| world.unregister_system(handler));
    }
}

fn manage_handlers<E>(
    mut handlers: Query<(Entity, &mut EventHandler<E>, &EventOf), Changed<EventHandler<E>>>,
    nodes: Query<&EventTarget>,
) -> Result
where
    E: FromWasmAbi + 'static,
{
    for (entity, mut handler, node) in &mut handlers {
        let node = nodes.get(node.0)?;
        let id = handler.handler;
        let function = Closure::new(move |ev: E| {
            let result = crate::web_runner::app_scope(|app| -> Result {
                let world = app.world_mut();
                world.run_system_with(
                    id,
                    Event {
                        entity,
                        event: SendWrapper::new(ev),
                    },
                )??;
                world.resource::<ScheduleTrigger>().trigger();
                Ok(())
            });

            if let Err(e) = result {
                bevy_log::error!("{e}");
            }
        });

        node.add_event_listener_with_callback(handler.event, function.as_ref().unchecked_ref())
            .js_err()?;

        handler.closure = Some(SendWrapper::new(function));
    }

    Ok(())
}
