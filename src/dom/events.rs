use crate::{js_err::JsErr, web_runner::ScheduleTrigger};

use super::{DomSystems, html::Node};
use bevy_app::prelude::*;
use bevy_ecs::{component::HookContext, prelude::*, system::SystemId, world::DeferredWorld};
use send_wrapper::SendWrapper;
use wasm_bindgen::{JsCast, convert::FromWasmAbi, prelude::Closure};

pub(super) struct EventsPlugin;

impl Plugin for EventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (
                OnClick::transform.in_set(DomSystems::Attach),
                manage_handlers::<web_sys::Event>.after(DomSystems::Attach),
            ),
        );
    }
}

pub type Ev<E> = In<Event<E>>;

#[derive(Clone)]
pub struct Event<E>(SendWrapper<E>);

impl<E> AsRef<E> for Event<E> {
    fn as_ref(&self) -> &E {
        &self.0
    }
}

impl<E> core::ops::Deref for Event<E> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

type Handler<E> = SystemId<In<Event<E>>>;
type ClickHandler = Handler<web_sys::Event>;

#[derive(Component)]
pub struct OnClick(Box<dyn FnOnce(&mut World) -> ClickHandler + Send + Sync>);

impl OnClick {
    pub fn new<S, M>(system: S) -> Self
    where
        S: IntoSystem<In<Event<web_sys::Event>>, (), M> + Send + Sync + 'static,
    {
        Self(Box::new(move |world: &mut World| {
            world.register_system(system)
        }))
    }

    fn transform(world: &mut World) {
        let mut clicks = world.query_filtered::<Entity, With<OnClick>>();
        let clicks: Vec<_> = clicks.iter(world).collect();

        for click in clicks {
            let handler = world.entity_mut(click).take::<OnClick>().unwrap();
            let id = (handler.0)(world);
            world.entity_mut(click).insert(EventHandler {
                handler: id,
                event: "click",
                closure: None,
            });
        }
    }
}

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

        let Some(node) = world.get::<Node>(context.entity) else {
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
    mut handlers: Query<(&mut EventHandler<E>, &Node), Changed<EventHandler<E>>>,
) -> Result
where
    E: FromWasmAbi + 'static,
{
    for (mut handler, node) in &mut handlers {
        let id = handler.handler;
        let function = Closure::new(move |ev: E| {
            let result = crate::web_runner::app_scope(|app| -> Result {
                let world = app.world_mut();
                world.run_system_with(id, Event(SendWrapper::new(ev)))?;
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

// fn call_handlers(world: &mut World) -> Result {
//     let queue = world.resource::<QueuedEvents>().0.clone();
//     let data: Vec<_> = std::mem::take(queue.lock().unwrap().as_mut());
//
//     for id in data {
//         world.run_system(id)?;
//     }
//
//     Ok(())
// }
