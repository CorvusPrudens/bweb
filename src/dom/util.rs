use crate::prelude::*;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use std::borrow::Cow;

pub(crate) struct UtilsPlugin;

impl Plugin for UtilsPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(SetTitle::observe);
    }
}

#[derive(Event)]
pub struct SetTitle(pub Cow<'static, str>);

impl SetTitle {
    pub fn new(title: impl Into<Cow<'static, str>>) -> Self {
        Self(title.into())
    }

    fn observe(
        ev: On<Self>,
        head: Single<Entity, With<Head>>,
        title: Query<Entity, With<Title>>,
        mut commands: Commands,
    ) {
        let value = ev.0.clone();

        if let Ok(title) = title.single() {
            commands
                .entity(title)
                .despawn_children()
                .with_child(Text::new(value));
        } else {
            commands.spawn((ChildOf(*head), Title, children![Text::new(value)]));
        }
    }
}
