use crate::dom::{DomSystems, html::Node};
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use send_wrapper::SendWrapper;
use wasm_bindgen::JsValue;

pub struct PropPlugin;

impl Plugin for PropPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (
                Value::resolve_props,
                Checked::resolve_props,
                Selected::resolve_props,
                TextContent::resolve_props,
            )
                .after(DomSystems::Insert)
                .before(DomSystems::Attach),
        );
    }
}

pub trait Prop {
    const NAME: &'static str;
    type Value: Into<JsValue> + Clone + Send + Sync + 'static;
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
        props: Query<(&Node, &Self), Or<(Changed<Node>, Changed<Self>)>>,
        mut target: Local<Option<SendWrapper<JsValue>>>,
    ) {
        let target = target.get_or_insert_with(|| SendWrapper::new(P::NAME.into()));

        for (node, prop) in props {
            if let Err(e) = js_sys::Reflect::set(node, target, &prop.0.clone().into()) {
                log::error!("failed to set property: {e:?}");
            }
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
