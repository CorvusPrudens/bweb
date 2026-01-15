use crate::dom::prelude::*;
use bevy_app::Plugin;
use bevy_ecs::prelude::*;

pub(crate) struct RelativeMousePlugin;

impl Plugin for RelativeMousePlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.add_observer(RelativeMouse::observe_insert)
            .add_observer(RelativeMouse::observe_replace);
    }
}

/// The position of the mouse relative to the this entity's HTML element.
///
/// This component is updated without triggering an ECS update, so should
/// not be solely relied upon for mouse position updates.
#[derive(Debug, Component, Default)]
pub struct RelativeMouse {
    pub x: f64,
    pub y: f64,
    event: Option<(Entity, Entity)>,
}

impl RelativeMouse {
    fn observe_insert(
        trigger: On<Insert, Self>,
        mut relative: Query<&mut RelativeMouse>,
        window: Single<Entity, With<Window>>,
        mut commands: Commands,
    ) -> Result {
        let target = trigger.entity;
        let mut relative = relative.get_mut(target)?;

        let event = move |ev: Ev<web_sys::PointerEvent>,
                          mut position: Query<(&Element, &mut RelativeMouse)>|
              -> Result {
            let (element, mut position) = position.get_mut(target)?;
            let (x, y) = relative_mouse_position(&ev, element);
            position.x = x;
            position.y = y;

            Ok(())
        };

        let on_move = ev::pointer_move(event).capturing().suppress();
        let on_down = ev::pointer_down(event).capturing().suppress();

        let on_move = commands.spawn((EventOf(*window), on_move)).id();
        let on_down = commands.spawn((EventOf(*window), on_down)).id();

        relative.event = Some((on_move, on_down));

        Ok(())
    }

    fn observe_replace(
        trigger: On<Replace, Self>,
        mut ev: Query<&mut Self>,
        mut commands: Commands,
    ) -> Result {
        let mut ev = ev.get_mut(trigger.entity)?;
        if let Some(ev) = ev.event.take() {
            commands.entity(ev.0).despawn();
            commands.entity(ev.1).despawn();
        }

        Ok(())
    }
}

pub fn relative_mouse_position(event: &web_sys::PointerEvent, element: &Element) -> (f64, f64) {
    let element_rect = element.get_bounding_client_rect();
    let x = event.page_x();
    let y = event.page_y();

    (x as f64 - element_rect.x(), y as f64 - element_rect.y())
}
