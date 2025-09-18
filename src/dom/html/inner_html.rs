use crate::dom::{DomSystems, html::Element};
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use std::borrow::Cow;

#[derive(Debug, Component, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InnerHtml(Cow<'static, str>);

impl core::ops::Deref for InnerHtml {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl core::fmt::Write for InnerHtml {
    fn write_str(&mut self, input: &str) -> core::fmt::Result {
        self.to_mut().push_str(input);

        Ok(())
    }
}

impl AsRef<str> for InnerHtml {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl InnerHtml {
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

    fn attach(attrs: Query<(&Self, Option<&Element>), Changed<Self>>) -> Result {
        for (html, element) in &attrs {
            let Some(element) = element else {
                return Err("`InnerHTML` property requires an Element".into());
            };

            element.set_inner_html(&html);
        }

        Ok(())
    }

    fn observe_remove(trigger: On<Remove, Self>, attr: Query<&Element>) {
        let Ok(element) = attr.get(trigger.target()) else {
            return;
        };

        element.set_inner_html("")
    }

    pub(super) fn plugin(app: &mut App) {
        app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
            .add_observer(Self::observe_remove);
    }
}
