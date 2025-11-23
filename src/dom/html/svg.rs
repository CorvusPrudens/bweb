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
#[derive(Debug, Component)]
pub struct SvgElementName(pub &'static str);

fn inject_svg_element(
    elements: Query<(Entity, &SvgElementName), Without<super::Node>>,
    document: Single<&Document>,
    mut commands: Commands,
) -> Result {
    for (entity, element) in &elements {
        let element: web_sys::SvgElement = document
            .create_element_ns(Some("http://www.w3.org/2000/svg"), element.0)
            .js_err()?
            .dyn_into()
            .unwrap();

        commands.entity(entity).insert((
            super::SvgElement(SendWrapper::new(element.clone())),
            super::Element(SendWrapper::new(element.clone().dyn_into().unwrap())),
            super::Node(SendWrapper::new(element.dyn_into().unwrap())),
        ));
    }

    Ok(())
}

#[derive(Debug, Default, Component, PartialEq, Eq, Clone)]
#[require(SvgElementName("svg"), Xmlns::new("http://www.w3.org/2000/svg"))]
pub struct Svg;

#[derive(Debug, Default, Component, PartialEq, Eq, Clone)]
#[require(SvgElementName("path"))]
pub struct Path;
