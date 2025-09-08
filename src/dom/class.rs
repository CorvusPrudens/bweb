use super::{DomSystems, html::Element};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use std::borrow::Cow;

pub(super) struct ClassPlugin;

impl Plugin for ClassPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, Class::attach_class.in_set(DomSystems::Attach))
            .add_observer(Class::observe_replace);
    }
}

#[derive(Debug, Component)]
#[relationship(relationship_target = Classes)]
pub struct ClassOf(pub Entity);

#[derive(Debug, Component)]
#[relationship_target(relationship = ClassOf, linked_spawn)]
pub struct Classes(Vec<Entity>);

#[doc(hidden)]
pub use bevy_ecs::spawn::Spawn;

#[macro_export]
macro_rules! class {
    [$($class:expr),*$(,)?] => {
        <$crate::dom::class::Classes>::spawn((
            $($crate::dom::class::Spawn(
                $crate::dom::class::Class::new($class)
            )),*
        ))
    };
}

#[macro_export]
macro_rules! classes {
    [$($class:expr),*$(,)?] => {
        <$crate::dom::class::Classes>::spawn((
            $($crate::dom::class::Spawn(
                $class
            )),*
        ))
    };
}

#[derive(Debug, Component, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Class(Cow<'static, str>);

impl Class {
    pub fn new(class: &'static str) -> Self {
        Self(Cow::Borrowed(class))
    }

    fn attach_class(
        texts: Query<(&Self, &ClassOf), Changed<Self>>,
        element: Query<&Element>,
    ) -> Result {
        for (class, parent) in &texts {
            element
                .get(parent.0)?
                .class_list()
                .add_1(&class.0)
                .js_err()?;
        }

        Ok(())
    }

    fn observe_replace(
        trigger: Trigger<OnReplace, Self>,
        class: Query<(&Self, &ClassOf)>,
        element: Query<&Element>,
    ) -> Result {
        let Ok((class, parent)) = class.get(trigger.target()) else {
            return Ok(());
        };
        let Ok(element) = element.get(parent.0) else {
            return Ok(());
        };

        element.class_list().remove_1(&class.0).js_err()?;

        Ok(())
    }
}
