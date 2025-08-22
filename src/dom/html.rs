use super::{Document, DomSystems};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::{component::HookContext, prelude::*, world::DeferredWorld};
use send_wrapper::SendWrapper;
use std::borrow::Cow;
use wasm_bindgen::JsCast;

pub(super) struct HtmlPlugin;

impl Plugin for HtmlPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreStartup,
            |document: Res<Document>, mut commands: Commands| {
                let body = document.body().expect("document body should be available");

                commands.spawn((
                    Body,
                    HtmlElement(SendWrapper::new(body.clone())),
                    Node(SendWrapper::new(body.dyn_into().unwrap())),
                ));
            },
        )
        .add_systems(
            PostUpdate,
            ((update_text, (inject_element, inject_text))
                .chain()
                .in_set(DomSystems::Insert),),
        );
    }
}

#[derive(Debug, Component)]
pub struct HtmlElement(SendWrapper<web_sys::HtmlElement>);

impl core::ops::Deref for HtmlElement {
    type Target = web_sys::HtmlElement;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Component)]
#[component(on_replace = Self::on_remove_hook)]
pub struct Node(SendWrapper<web_sys::Node>);

impl core::ops::Deref for Node {
    type Target = web_sys::Node;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Node {
    fn on_remove_hook(world: DeferredWorld, context: HookContext) {
        let element = world.get::<Node>(context.entity).unwrap();
        if let Some(element) = element.0.dyn_ref::<web_sys::Element>() {
            element.remove();
        }
    }
}

/// An HTML element inserter.
#[derive(Debug, Component)]
pub struct Element(pub &'static str);

fn inject_element(
    elements: Query<(Entity, &Element), Without<Node>>,
    document: Res<Document>,
    mut commands: Commands,
) -> Result {
    for (entity, element) in &elements {
        let element: web_sys::HtmlElement = document
            .create_element(element.0)
            .js_err()?
            .dyn_into()
            .unwrap();

        commands.entity(entity).insert((
            HtmlElement(SendWrapper::new(element.clone())),
            Node(SendWrapper::new(element.dyn_into().unwrap())),
        ));
    }

    Ok(())
}

#[derive(Debug, Component)]
#[require(Element("a"))]
pub struct A;

#[derive(Debug, Component)]
#[require(Element("video"))]
pub struct Video;

#[derive(Debug, Component)]
#[require(Element("body"))]
pub struct Body;

#[derive(Debug, Component)]
#[require(Element("div"))]
pub struct Div;

#[derive(Debug, Component)]
#[require(Element("nav"))]
pub struct Nav;

#[derive(Debug, Component)]
#[require(Element("header"))]
pub struct Header;

#[derive(Debug, Component)]
#[require(Element("footer"))]
pub struct Footer;

#[derive(Debug, Component)]
#[require(Element("main"))]
pub struct Main;

#[derive(Debug, Component)]
#[require(Element("button"))]
pub struct Button;

#[derive(Debug, Component)]
pub struct Text(Cow<'static, str>);

impl Text {
    pub fn new(text: &'static str) -> Self {
        Self(Cow::Borrowed(text))
    }

    pub fn dynamic(text: String) -> Self {
        Self(Cow::Owned(text))
    }
}

fn inject_text(
    texts: Query<(Entity, &Text), Without<Node>>,
    document: Res<Document>,
    mut commands: Commands,
) -> Result {
    for (entity, text) in &texts {
        let element: web_sys::Node = document.create_text_node(&text.0).dyn_into().unwrap();
        commands
            .entity(entity)
            .insert(Node(SendWrapper::new(element)));
    }

    Ok(())
}

fn update_text(texts: Query<(&Text, &Node), Changed<Text>>) {
    for (text, node) in &texts {
        let node: &web_sys::Text = node.0.dyn_ref().unwrap();
        node.set_data(&text.0);
    }
}
