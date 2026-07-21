use crate::{
    dom::{DomSystems, prelude::attr::Xmlns},
    js_err::JsErr,
};
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use send_wrapper::SendWrapper;
use wasm_bindgen::JsCast;

use super::Document;

pub(super) struct SvgPlugin;

impl Plugin for SvgPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, inject_svg_element.in_set(DomSystems::Insert));
    }
}

/// An SVG element inserter.
#[derive(Component)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[cfg_attr(feature = "reflect", derive(bevy_reflect::Reflect))]
#[cfg_attr(feature = "reflect", reflect(Component))]
#[component(on_replace = Self::on_replace_hook)]
pub struct SvgElementName(pub &'static str);

impl SvgElementName {
    /// Drop the element this name created, mirroring
    /// [`HtmlElementName`](super::HtmlElementName).
    fn on_replace_hook(
        mut world: bevy_ecs::world::DeferredWorld,
        context: bevy_ecs::lifecycle::HookContext,
    ) {
        if let Ok(mut entity) = world.commands().get_entity(context.entity) {
            entity.try_remove::<(
                super::Node,
                super::EventTarget,
                super::Element,
                super::SvgElement,
                super::HtmlElement,
            )>();
        }
    }
}

fn inject_svg_element(
    elements: Query<(Entity, &SvgElementName), Without<super::Node>>,
    document: Single<&Document>,
    mut commands: Commands,
) -> Result {
    for (entity, element) in &elements {
        let element = document
            .create_element_ns(Some("http://www.w3.org/2000/svg"), element.0)
            .js_err()?;

        commands.entity(entity).insert((
            super::Element(SendWrapper::new(element.clone())),
            super::SvgElement(SendWrapper::new(element.clone().unchecked_into())),
            super::Node(SendWrapper::new(element.unchecked_into())),
        ));
    }

    Ok(())
}

#[derive(Default, Component, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(SvgElementName("svg"), Xmlns::new("http://www.w3.org/2000/svg"))]
pub struct Svg;

#[derive(Default, Component, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(SvgElementName("path"))]
pub struct Path;
