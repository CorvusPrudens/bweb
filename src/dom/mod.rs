use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use send_wrapper::SendWrapper;

pub mod attributes;
pub mod class;
pub mod events;
pub mod html;

pub struct DomPlugin;

impl Plugin for DomPlugin {
    fn build(&self, app: &mut App) {
        let window = web_sys::window().expect("browser window should be available");
        let document = window
            .document()
            .expect("browser document should be available");

        app.insert_resource(Window(SendWrapper::new(window)))
            .insert_resource(Document(SendWrapper::new(document)))
            .add_plugins((
                events::EventsPlugin,
                class::ClassPlugin,
                html::HtmlPlugin,
                attributes::AttributePlugin,
            ))
            .configure_sets(
                PostUpdate,
                (
                    DomSystems::Insert,
                    DomSystems::Reparent.after(DomSystems::Insert),
                    DomSystems::Attach.after(DomSystems::Reparent),
                ),
            )
            .add_systems(PostUpdate, (reparent.chain().in_set(DomSystems::Reparent),));
    }
}

#[derive(Resource)]
pub struct Window(SendWrapper<web_sys::Window>);

impl core::ops::Deref for Window {
    type Target = web_sys::Window;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Resource)]
pub struct Document(SendWrapper<web_sys::Document>);

impl core::ops::Deref for Document {
    type Target = web_sys::Document;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(SystemSet, Clone, PartialEq, Eq, Debug, Hash)]
pub enum DomSystems {
    /// Insert nodes into the document.
    Insert,
    /// Re-parent newly spawned or modified hierarchies.
    Reparent,
    /// Attach events, classes, or attributes to elements.
    Attach,
}

fn reparent(
    body: Query<Entity, With<html::Body>>,
    nodes: Query<(Ref<html::Node>, Option<&Children>)>,
) -> Result {
    fn handle_children(
        nodes: &Query<(Ref<html::Node>, Option<&Children>)>,
        parent_entity: Entity,
    ) -> Result {
        let (node, children) = nodes.get(parent_entity)?;

        for child_entity in children.iter().flat_map(|c| c.iter()) {
            let Ok((child_node, _)) = nodes.get(child_entity) else {
                continue;
            };

            if node.is_changed() || child_node.is_changed() {
                node.append_child(&child_node).js_err()?;
            }

            handle_children(nodes, child_entity)?;
        }

        Ok(())
    }

    let body = body.single()?;
    handle_children(&nodes, body)?;

    Ok(())
}

pub mod prelude {
    pub use super::attributes::*;
    pub use super::events::*;
    pub use super::html::*;
    pub use crate::class;
}
