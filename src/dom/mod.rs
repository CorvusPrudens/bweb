use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;

pub mod attr;
pub mod class;
pub mod events;
pub mod html;
pub mod prop;
pub mod util;

#[derive(Default)]
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct DomPlugin;

impl Plugin for DomPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            events::EventsPlugin,
            class::ClassPlugin,
            html::HtmlPlugin,
            attr::AttributePlugin,
            prop::PropPlugin,
            util::UtilsPlugin,
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
        .add_systems(PostUpdate, (reparent.in_set(DomSystems::Reparent),));
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
    html: Query<Entity, With<html::elements::Html>>,
    nodes: Query<(Ref<html::Node>, Option<Ref<Children>>)>,
    lookup: html::NodeLookup,
) -> Result {
    fn handle_children(
        nodes: &Query<(Ref<html::Node>, Option<Ref<Children>>)>,
        lookup: &html::NodeLookup,
        parent_entity: Entity,
    ) -> Result {
        let (node, children) = nodes.get(parent_entity)?;

        let Some(children) = children else {
            return Ok(());
        };

        let children_changed = children.is_changed();
        let children: &[Entity] = children.into_inner().as_ref();
        for (i, child_entity) in children.iter().enumerate() {
            let Ok((child_node, _)) = nodes.get(*child_entity) else {
                continue;
            };

            if node.is_changed() {
                node.append_child(&child_node).js_err()?;
            } else if !children_changed && child_node.is_changed() {
                // look for the next child (that we're aware of)
                // that's a child of the parent node
                let next = children[i + 1..].iter().find_map(|c| {
                    let (child_node, ..) = nodes.get(*c).ok()?;

                    node.contains(Some(&child_node)).then_some(child_node)
                });

                match next {
                    Some(next) => {
                        node.insert_before(&child_node, Some(&next)).js_err()?;
                    }
                    None => {
                        node.append_child(&child_node).js_err()?;
                    }
                }
            }

            handle_children(nodes, lookup, *child_entity)?;
        }

        if children_changed && !node.is_changed() {
            sync_child_order(node.into_inner(), children, nodes, lookup)?;
        }

        Ok(())
    }

    let html = html.single()?;
    handle_children(&nodes, &lookup, html)?;

    Ok(())
}

/// Make the DOM order of `parent`'s entity-backed children match their
/// `Children` order, attaching any that aren't in the DOM yet. Nodes on a
/// longest increasing subsequence of the current order stay put, so the
/// number of `insert_before` calls (each a remove+insert that drops focus
/// and restarts animations on the moved node) is minimal. When the DOM
/// already matches, this only reads sibling pointers.
fn sync_child_order(
    parent: &html::Node,
    children: &[Entity],
    nodes: &Query<(Ref<html::Node>, Option<Ref<Children>>)>,
    lookup: &html::NodeLookup,
) -> Result {
    use bevy_platform::collections::{HashMap, HashSet};

    // The desired order: entity children that have DOM nodes. Children
    // whose nodes don't exist yet are picked up by a later run once
    // injection inserts their `Node`.
    let mut desired = Vec::with_capacity(children.len());
    let mut desired_index = HashMap::with_capacity(children.len());
    for child in children {
        let Ok((child_node, _)) = nodes.get(*child) else {
            continue;
        };

        desired_index.insert(*child, desired.len());
        desired.push((**child_node).clone());
    }

    // The current order, as desired-indices of the parent's DOM children.
    // Foreign nodes (not entity-backed, or not in `children`) are skipped;
    // moves are anchored on managed nodes only, so they stay where they are.
    let mut current = Vec::with_capacity(desired.len());
    let mut dom_child = parent.first_child();
    while let Some(node) = dom_child {
        if let Some(index) = lookup.get(&node).and_then(|e| desired_index.get(&e)) {
            current.push(*index);
        }

        dom_child = node.next_sibling();
    }

    let in_order = current.len() == desired.len() && current.is_sorted();
    if in_order {
        return Ok(());
    }

    let keep: HashSet<usize> = longest_increasing_subsequence(&current)
        .into_iter()
        .collect();

    let mut anchor: Option<web_sys::Node> = None;
    for (i, node) in desired.iter().enumerate().rev() {
        if keep.contains(&i) {
            anchor = Some(node.clone());
            continue;
        }

        parent.insert_before(node, anchor.as_ref()).js_err()?;
        anchor = Some(node.clone());
    }

    Ok(())
}

/// The values of one longest strictly increasing subsequence of `seq`.
/// `seq` must not contain duplicates.
fn longest_increasing_subsequence(seq: &[usize]) -> Vec<usize> {
    // Patience sorting: `tails[k]` is the position in `seq` of the smallest
    // value ending an increasing subsequence of length `k + 1`.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; seq.len()];

    for (i, &value) in seq.iter().enumerate() {
        let len = tails.partition_point(|&tail| seq[tail] < value);
        if len > 0 {
            prev[i] = Some(tails[len - 1]);
        }

        if len == tails.len() {
            tails.push(i);
        } else {
            tails[len] = i;
        }
    }

    let mut values = Vec::with_capacity(tails.len());
    let mut position = tails.last().copied();
    while let Some(i) = position {
        values.push(seq[i]);
        position = prev[i];
    }

    values.reverse();
    values
}

pub mod prelude {
    pub use super::attr;
    pub use super::class::*;
    pub use super::events::*;
    pub use super::html::NodeLookup;
    pub use super::html::{elements::*, svg::*, *};
    pub use super::prop;
    pub use super::util::*;
    pub use crate::{class, classes, events};
}

#[cfg(test)]
mod test {
    use super::longest_increasing_subsequence as lis;

    #[test]
    fn lis_basic() {
        assert_eq!(lis(&[]), Vec::<usize>::new());
        assert_eq!(lis(&[3]), vec![3]);
        assert_eq!(lis(&[0, 1, 2, 3]), vec![0, 1, 2, 3]);
        assert_eq!(lis(&[3, 2, 1, 0]).len(), 1);
        assert_eq!(lis(&[2, 0, 1, 3]), vec![0, 1, 3]);
        assert_eq!(lis(&[1, 2, 0, 3]), vec![1, 2, 3]);
    }

    #[test]
    fn lis_is_increasing_subsequence() {
        let seq = [5, 0, 3, 1, 6, 2, 7, 4];
        let result = lis(&seq);

        assert!(result.is_sorted());
        // Result is a subsequence: values appear in `seq` in the same order.
        let mut remaining = seq.iter();
        for value in &result {
            assert!(remaining.any(|v| v == value));
        }
        // [0, 1, 2, 4] and [0, 1, 2, 7] etc. are the maxima here.
        assert_eq!(result.len(), 4);
    }
}
