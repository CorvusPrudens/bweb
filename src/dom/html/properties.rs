use crate::dom::{DomSystems, html::HtmlInputElement};
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use std::borrow::Cow;

pub(super) struct PropertyPlugin;

impl Plugin for PropertyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            ValueProp::update_values.in_set(DomSystems::Attach),
        );
    }
}

#[derive(Component)]
pub struct ValueProp(pub Cow<'static, str>);

impl ValueProp {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Self {
        Self(value.into())
    }

    fn update_values(
        values: Query<
            (&HtmlInputElement, &ValueProp),
            Or<(Changed<ValueProp>, Changed<HtmlInputElement>)>,
        >,
    ) {
        for (element, prop) in &values {
            element.set_value(&prop.0);
        }
    }
}
