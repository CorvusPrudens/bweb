use super::{DomSystems, html::HtmlElement};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use std::borrow::Cow;

pub(super) struct AttributePlugin;

impl Plugin for AttributePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            Href::plugin,
            Title::plugin,
            Style::plugin,
            Width::plugin,
            Height::plugin,
            Src::plugin,
            Muted::plugin,
            Autoplay::plugin,
            Loop::plugin,
            Disabled::plugin,
        ));
    }
}

macro_rules! attribute {
    ($ty:ident, $attr:literal) => {
        #[derive(Debug, Component)]
        pub struct $ty(Cow<'static, str>);

        impl $ty {
            pub fn new(attribute: &'static str) -> Self {
                Self(Cow::Borrowed(attribute))
            }

            // TODO: these should really be trait-like
            fn attach(attrs: Query<(&Self, Option<&HtmlElement>), Changed<Self>>) -> Result {
                for (href, element) in &attrs {
                    let Some(element) = element else {
                        return Err(
                            format!("'{}' attribute requires an HTML Element", $attr).into()
                        );
                    };

                    element.set_attribute($attr, &href.0).js_err()?;
                }

                Ok(())
            }

            fn observe_replace(
                trigger: Trigger<OnReplace, Self>,
                attr: Query<&HtmlElement>,
            ) -> Result {
                let Ok(element) = attr.get(trigger.target()) else {
                    return Ok(());
                };

                element.remove_attribute($attr).js_err()
            }

            fn plugin(app: &mut App) {
                app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
                    .add_observer(Self::observe_replace);
            }
        }
    };
}

attribute! {Href, "href"}
attribute! {Title, "title"}
attribute! {Style, "style"}
attribute! {Width, "width"}
attribute! {Height, "height"}
attribute! {Src, "src"}

macro_rules! boolean_attribute {
    ($ty:ident, $attr:literal) => {
        #[derive(Debug, Component)]
        pub struct $ty;

        impl $ty {
            // TODO: these should really be trait-like
            fn attach(attrs: Query<Option<&HtmlElement>, (Changed<Self>, With<Self>)>) -> Result {
                for element in &attrs {
                    let Some(element) = element else {
                        return Err(
                            format!("'{}' attribute requires an HTML Element", $attr).into()
                        );
                    };

                    element.set_attribute($attr, "").js_err()?;
                }

                Ok(())
            }

            fn observe_replace(
                trigger: Trigger<OnReplace, Self>,
                attr: Query<&HtmlElement>,
            ) -> Result {
                let Ok(element) = attr.get(trigger.target()) else {
                    return Ok(());
                };

                element.remove_attribute($attr).js_err()
            }

            fn plugin(app: &mut App) {
                app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
                    .add_observer(Self::observe_replace);
            }
        }
    };
}

boolean_attribute! {Muted, "muted"}
boolean_attribute! {Autoplay, "autoplay"}
boolean_attribute! {Loop, "loop"}
boolean_attribute! {Disabled, "disabled"}
