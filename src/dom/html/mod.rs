use super::{DomStartupSystems, DomSystems};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::{component::HookContext, prelude::*, world::DeferredWorld};
use send_wrapper::SendWrapper;
use std::borrow::Cow;
use wasm_bindgen::JsCast;

pub mod elements;
pub mod svg;

pub(super) struct HtmlPlugin;

impl Plugin for HtmlPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(svg::SvgPlugin)
            .add_systems(
                PreStartup,
                initialize_window.in_set(DomStartupSystems::Window),
            )
            .add_systems(
                PostUpdate,
                ((update_text, (inject_element, inject_text))
                    .chain()
                    .in_set(DomSystems::Insert),),
            );
    }
}

macro_rules! web_wrapper {
    ($ty:ident) => {
        #[derive(Debug, Component)]
        pub struct $ty(SendWrapper<web_sys::$ty>);

        impl core::ops::Deref for $ty {
            type Target = web_sys::$ty;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl AsRef<web_sys::$ty> for $ty {
            fn as_ref(&self) -> &web_sys::$ty {
                &self.0
            }
        }
    };
}

web_wrapper!(Window);
web_wrapper!(Document);

fn initialize_window(mut commands: Commands) -> Result {
    let window = web_sys::window().ok_or("browser window should be available")?;
    let window_target: &web_sys::EventTarget = &window;

    commands.spawn((
        EventTarget(SendWrapper::new(window_target.clone())),
        Window(SendWrapper::new(window.clone())),
    ));

    let document = window
        .document()
        .ok_or("browser document should be available")?;
    let document_node: &web_sys::Node = &document;

    commands.spawn((
        Node(SendWrapper::new(document_node.clone())),
        Document(SendWrapper::new(document.clone())),
    ));

    let html = document
        .document_element()
        .ok_or("document HTML should be available")?;
    let html = commands
        .spawn((
            elements::Html,
            HtmlElement(SendWrapper::new(html.clone().dyn_into().unwrap())),
            Element(SendWrapper::new(html.clone().dyn_into().unwrap())),
            Node(SendWrapper::new(html.dyn_into().unwrap())),
        ))
        .id();

    let head = document.head().ok_or("document head should be available")?;
    commands.spawn((
        ChildOf(html),
        elements::Head,
        HtmlElement(SendWrapper::new(head.clone().dyn_into().unwrap())),
        Element(SendWrapper::new(head.clone().dyn_into().unwrap())),
        Node(SendWrapper::new(head.dyn_into().unwrap())),
    ));

    let body = document.body().ok_or("document body should be available")?;

    commands.spawn((
        ChildOf(html),
        elements::Body,
        HtmlElement(SendWrapper::new(body.clone())),
        Element(SendWrapper::new(body.clone().dyn_into().unwrap())),
        Node(SendWrapper::new(body.dyn_into().unwrap())),
    ));

    Ok(())
}

web_wrapper!(HtmlElement);
web_wrapper!(Element);
web_wrapper!(EventTarget);
web_wrapper!(SvgElement);

#[derive(Debug, Component)]
#[component(on_replace = Self::on_remove_hook, on_insert = Self::on_insert_hook)]
pub struct Node(SendWrapper<web_sys::Node>);

impl core::ops::Deref for Node {
    type Target = web_sys::Node;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<web_sys::Node> for Node {
    fn as_ref(&self) -> &web_sys::Node {
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

    fn on_insert_hook(mut world: DeferredWorld, context: HookContext) {
        let element = world.get::<Node>(context.entity).unwrap();
        let event_target: &web_sys::EventTarget = element.0.as_ref();
        let event_target = event_target.clone();

        world
            .commands()
            .entity(context.entity)
            .insert(EventTarget(SendWrapper::new(event_target)));
    }
}

/// An HTML element inserter.
#[derive(Debug, Component)]
pub struct HtmlElementName(pub &'static str);

fn inject_element(
    elements: Query<(Entity, &HtmlElementName), Without<Node>>,
    document: Single<&Document>,
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
            Element(SendWrapper::new(element.clone().dyn_into().unwrap())),
            Node(SendWrapper::new(element.dyn_into().unwrap())),
        ));
    }

    Ok(())
}

#[derive(Debug, Component, Clone, PartialEq, Eq)]
pub struct Text(Cow<'static, str>);

impl Text {
    pub fn new(text: impl Into<Cow<'static, str>>) -> Self {
        Self(text.into())
    }
}

fn inject_text(
    texts: Query<(Entity, &Text), Without<Node>>,
    document: Single<&Document>,
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
