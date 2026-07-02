use crate::dom::{
    DomSystems,
    prelude::attr::Xmlns,
    registry::{DomCommandBuffer, NodeIds},
};
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;

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
pub struct SvgElementName(pub &'static str);

fn inject_svg_element(
    elements: Query<(Entity, &SvgElementName), Without<super::Node>>,
    mut ids: ResMut<NodeIds>,
    mut buffer: ResMut<DomCommandBuffer>,
    mut commands: Commands,
) {
    for (entity, element) in &elements {
        let id = ids.alloc();
        buffer.create_element_ns(id, "http://www.w3.org/2000/svg", element.0, entity);

        commands.entity(entity).insert((
            id,
            super::Element::lazy(id),
            super::SvgElement::lazy(id),
            super::Node::lazy(id),
        ));
    }
}

#[derive(Default, Component, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(SvgElementName("svg"), Xmlns::new("http://www.w3.org/2000/svg"))]
pub struct Svg;

#[derive(Default, Component, PartialEq, Eq, Clone)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(SvgElementName("path"))]
pub struct Path;
