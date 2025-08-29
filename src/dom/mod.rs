use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;

pub mod attributes;
pub mod class;
pub mod events;
pub mod html;

pub struct DomPlugin;

impl Plugin for DomPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            events::EventsPlugin,
            class::ClassPlugin,
            html::HtmlPlugin,
            attributes::AttributePlugin,
        ))
        .configure_sets(
            PreStartup,
            (
                DomStartupSystems::Window,
                DomStartupSystems::Pathname.after(DomStartupSystems::Window),
            ),
        )
        .configure_sets(
            PostUpdate,
            (
                DomSystems::ResolveRoutes,
                DomSystems::Insert.after(DomSystems::ResolveRoutes),
                DomSystems::Reparent.after(DomSystems::Insert),
                DomSystems::Attach.after(DomSystems::Reparent),
            ),
        )
        .add_systems(PostUpdate, (reparent.chain().in_set(DomSystems::Reparent),));
    }
}

#[derive(SystemSet, Clone, PartialEq, Eq, Debug, Hash)]
pub enum DomStartupSystems {
    /// Set up the window and any default HTML elements.
    Window,
    /// Fetch the pathname.
    Pathname,
}

#[derive(SystemSet, Clone, PartialEq, Eq, Debug, Hash)]
pub enum DomSystems {
    /// Resolve routes following pathname changes.
    ResolveRoutes,
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

        let mut child_iter = children.iter().flat_map(|c| c.iter()).peekable();

        while let Some(child_entity) = child_iter.next() {
            let Ok((child_node, _)) = nodes.get(child_entity) else {
                continue;
            };

            if node.is_changed() {
                node.append_child(&child_node).js_err()?;
            } else if child_node.is_changed() {
                match child_iter.peek().and_then(|c| nodes.get(*c).ok()) {
                    Some((next, _)) => {
                        node.insert_before(&child_node, Some(&next)).js_err()?;
                    }
                    None => {
                        node.append_child(&child_node).js_err()?;
                    }
                }
            }

            handle_children(nodes, child_entity)?;
        }

        // for child_entity in children.iter().flat_map(|c| c.iter()) {
        //     let Ok((child_node, _)) = nodes.get(child_entity) else {
        //         continue;
        //     };
        //
        //     if node.is_changed() {
        //         node.append_child(&child_node).js_err()?;
        //     } else if child_node.is_changed() {
        //         node.append_child(&child_node).js_err()?;
        //     }
        //
        //     handle_children(nodes, child_entity)?;
        // }

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
    pub use crate::{class, events};
}
