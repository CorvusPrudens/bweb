use super::{DomSystems, html::Element};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use std::borrow::Cow;

pub(super) struct AttributePlugin;

impl Plugin for AttributePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            (
                Href::plugin,
                Title::plugin,
                Style::plugin,
                Width::plugin,
                Height::plugin,
                Src::plugin,
                Target::plugin,
                Tabindex::plugin,
                Draggable::plugin,
            ),
            (
                D::plugin,
                Lang::plugin,
                ViewBox::plugin,
                Xmlns::plugin,
                Fill::plugin,
                Muted::plugin,
                Autoplay::plugin,
                Loop::plugin,
                Disabled::plugin,
                Download::plugin,
            ),
            (Hidden::plugin,),
        ));
    }
}

macro_rules! attribute {
    ($ty:ident, $attr:literal) => {
        #[derive(Debug, Component, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $ty(Cow<'static, str>);

        impl core::ops::Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl core::fmt::Write for $ty {
            fn write_str(&mut self, input: &str) -> core::fmt::Result {
                self.to_mut().push_str(input);

                Ok(())
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl $ty {
            pub fn new(attribute: impl Into<Cow<'static, str>>) -> Self {
                Self(attribute.into())
            }

            pub fn clear(&mut self) {
                match &mut self.0 {
                    Cow::Borrowed(_) => {
                        self.0 = Cow::Owned(String::new());
                    }
                    Cow::Owned(o) => {
                        o.clear();
                    }
                }
            }

            pub fn to_mut(&mut self) -> &mut String {
                self.0.to_mut()
            }

            // TODO: these should really be trait-like
            fn attach(attrs: Query<(&Self, Option<&Element>), Changed<Self>>) -> Result {
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

            fn observe_remove(trigger: On<Remove, Self>, attr: Query<&Element>) -> Result {
                let Ok(element) = attr.get(trigger.entity) else {
                    return Ok(());
                };

                element.remove_attribute($attr).js_err()
            }

            fn plugin(app: &mut App) {
                app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
                    .add_observer(Self::observe_remove);
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
attribute! {Target, "target"}
attribute! {Tabindex, "tabindex"}
attribute! {Draggable, "draggable"}
attribute! {D, "d"}
attribute! {Lang, "lang"}
attribute! {ViewBox, "viewBox"}
attribute! {Xmlns, "xmlns"}
attribute! {Fill, "fill"}

macro_rules! boolean_attribute {
    ($ty:ident, $attr:literal) => {
        #[derive(Debug, Component, Clone, PartialEq, Eq)]
        pub struct $ty;

        impl $ty {
            // TODO: these should really be trait-like
            fn attach(attrs: Query<Option<&Element>, (Changed<Self>, With<Self>)>) -> Result {
                for element in &attrs {
                    let Some(element) = element else {
                        return Err(format!("'{}' attribute requires a DOM Element", $attr).into());
                    };

                    element.set_attribute($attr, "").js_err()?;
                }

                Ok(())
            }

            fn observe_remove(trigger: On<Remove, Self>, attr: Query<&Element>) -> Result {
                let Ok(element) = attr.get(trigger.entity) else {
                    return Ok(());
                };

                element.remove_attribute($attr).js_err()
            }

            fn plugin(app: &mut App) {
                app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
                    .add_observer(Self::observe_remove);
            }
        }
    };
}

boolean_attribute! {Muted, "muted"}
boolean_attribute! {Autoplay, "autoplay"}
boolean_attribute! {Loop, "loop"}
boolean_attribute! {Disabled, "disabled"}
boolean_attribute! {Download, "download"}

// Enumerated attributes
#[derive(Debug, Component, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Hidden {
    Hidden,
    UntilFound,
}

impl Hidden {
    fn attach(attrs: Query<(&Self, Option<&Element>), Changed<Self>>) -> Result {
        for (attr, element) in &attrs {
            let Some(element) = element else {
                return Err("'hidden' attribute requires a DOM Element".into());
            };

            match attr {
                Self::Hidden => {
                    element.set_attribute("hidden", "hidden").js_err()?;
                }
                Self::UntilFound => {
                    element.set_attribute("hidden", "until-found").js_err()?;
                }
            }
        }

        Ok(())
    }

    fn observe_remove(trigger: On<Remove, Self>, attr: Query<&Element>) -> Result {
        let Ok(element) = attr.get(trigger.entity) else {
            return Ok(());
        };

        element.remove_attribute("hidden").js_err()
    }

    fn plugin(app: &mut App) {
        app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
            .add_observer(Self::observe_remove);
    }
}
