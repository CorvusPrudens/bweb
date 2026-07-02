use crate::dom::{
    DomSystems,
    html::Node,
    registry::{DomCommandBuffer, NodeId},
};
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;

pub struct PropPlugin;

impl Plugin for PropPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (
                Value::resolve_props,
                Checked::resolve_props,
                TextContent::resolve_props,
            )
                .after(DomSystems::Insert)
                .before(DomSystems::Attach),
        );
    }
}

pub trait Prop {
    const NAME: &'static str;
    type Value: PropValue;
}

/// A value assignable to a JS property through the DOM command stream.
pub trait PropValue: Clone + Send + Sync + 'static {
    fn push(&self, id: NodeId, name: &str, buffer: &mut DomCommandBuffer);
}

impl PropValue for String {
    fn push(&self, id: NodeId, name: &str, buffer: &mut DomCommandBuffer) {
        buffer.set_property_str(id, name, self);
    }
}

impl PropValue for bool {
    fn push(&self, id: NodeId, name: &str, buffer: &mut DomCommandBuffer) {
        buffer.set_property_bool(id, name, *self);
    }
}

#[derive(Component)]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct PropContainer<P: Prop>(pub P::Value);

impl<P: Prop> Clone for PropContainer<P>
where
    P::Value: Clone,
{
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<P: Prop> PartialEq for PropContainer<P>
where
    P::Value: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl<P: Prop + 'static> PropContainer<P> {
    fn resolve_props(
        props: Query<(&NodeId, &Self), Or<(Changed<Node>, Changed<Self>)>>,
        mut buffer: ResMut<DomCommandBuffer>,
    ) {
        for (id, prop) in props {
            prop.0.push(*id, P::NAME, &mut buffer);
        }
    }

    pub fn new(value: P::Value) -> Self {
        Self(value)
    }
}

macro_rules! prop {
    ($full_ident:ident, $short_ident:ident, $name:literal, $value:ty) => {
        #[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
        #[cfg_attr(feature = "debug", derive(Debug))]
        pub struct $full_ident;

        impl Prop for $full_ident {
            const NAME: &'static str = $name;
            type Value = $value;
        }

        pub type $short_ident = PropContainer<$full_ident>;
    };
}

prop!(ValueProp, Value, "value", String);
prop!(CheckedProp, Checked, "checked", bool);
prop!(SelectedProp, Selected, "selected", bool);
prop!(TextContentProp, TextContent, "textContent", String);
