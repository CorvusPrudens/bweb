use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::{prelude::*, system::SystemParam};

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
        .add_systems(
            PostUpdate,
            reparent_incremental.in_set(DomSystems::Reparent),
        );
    }
}

/// The entity-backed children this entity's node currently has attached in
/// the DOM, in DOM order (foreign, non-entity nodes are never tracked).
/// Maintained in lockstep with every structural DOM call so reconciliation
/// can re-derive the current order without reading the DOM back — each
/// `first_child`/`next_sibling`/`contains` read is a Wasm↔JS crossing.
#[derive(Component, Default)]
#[cfg_attr(feature = "debug", derive(Debug))]
pub(crate) struct DomChildren(pub(crate) Vec<Entity>);

/// The entity under whose node this entity's node is currently attached, if
/// any — the back-pointer that keeps [`DomChildren`] updates cheap on detach.
#[derive(Component, Default)]
#[cfg_attr(feature = "debug", derive(Debug))]
pub(crate) struct DomParent(pub(crate) Option<Entity>);

/// The ECS-side mirror of the DOM structure bweb has built. Every
/// `append_child`/`insert_before`/removal of a managed node must be paired
/// with the matching mirror update (removals happen in `Node::on_remove_hook`
/// and `remove_text`), keeping [`DomChildren`]/[`DomParent`] exact. This
/// assumes bweb is the only thing mutating the structure of managed nodes.
#[derive(SystemParam)]
struct DomMirror<'w, 's> {
    children: Query<'w, 's, &'static mut DomChildren>,
    parents: Query<'w, 's, &'static mut DomParent>,
}

impl DomMirror<'_, '_> {
    /// Whether `child`'s node is currently attached under `parent`'s node.
    fn is_attached_to(&self, child: Entity, parent: Entity) -> bool {
        self.parents
            .get(child)
            .is_ok_and(|dom_parent| dom_parent.0 == Some(parent))
    }

    /// Record that `child` was just placed under `parent`, immediately before
    /// `anchor` (`None` = appended at the end), detaching it from wherever
    /// the DOM implicitly moved it from.
    fn attach(&mut self, child: Entity, parent: Entity, anchor: Option<Entity>) -> Result {
        let mut dom_parent = self.parents.get_mut(child)?;
        let previous = dom_parent.0.replace(parent);
        // The back-pointer says which mirror (if any) holds the child, so the
        // O(len) de-duplicating scan only runs on a same-parent reposition.
        let repositioning = previous == Some(parent);
        if let Some(previous) = previous
            && previous != parent
            && let Ok(mut siblings) = self.children.get_mut(previous)
        {
            siblings.0.retain(|&sibling| sibling != child);
        }

        let mut siblings = self.children.get_mut(parent)?;
        if repositioning {
            siblings.0.retain(|&sibling| sibling != child);
        }
        let index = match anchor {
            Some(anchor) => siblings
                .0
                .iter()
                .position(|&sibling| sibling == anchor)
                .ok_or("DOM mirror desync: insert_before anchor not under parent")?,
            None => siblings.0.len(),
        };
        siblings.0.insert(index, child);

        Ok(())
    }
}

fn reparent_incremental(
    changed_nodes: Query<(Entity, &html::Node, Option<Ref<Children>>), Changed<html::Node>>,
    changed_children: Query<(Entity, Ref<html::Node>, &Children), Changed<Children>>,
    nodes: Query<(Ref<html::Node>, Option<Ref<Children>>)>,
    parents: Query<&ChildOf>,
    mut mirror: DomMirror,
) -> Result {
    for (entity, node, children) in &changed_nodes {
        // Attach every child onto this fresh node, in `Children`
        // order. A child whose own node isn't created yet is skipped here and
        // picked up lower down.
        if let Some(children) = children {
            let children: &[Entity] = children.into_inner().as_ref();
            let mut attached = Vec::with_capacity(children.len());
            for &child in children {
                if let Ok((child_node, _)) = nodes.get(child) {
                    node.append_child(&child_node).js_err()?;
                    attached.push(child);
                }
            }

            // A changed `Node` is brand new (or a replacement), so exactly
            // the children appended above are under it: rebuild its mirror
            // wholesale.
            for &child in &attached {
                let mut dom_parent = mirror.parents.get_mut(child)?;
                let previous = dom_parent.0.replace(entity);
                if let Some(previous) = previous
                    && previous != entity
                    && let Ok(mut siblings) = mirror.children.get_mut(previous)
                {
                    siblings.0.retain(|&sibling| sibling != child);
                }
            }
            let mut own = mirror.children.get_mut(entity)?;
            let old = core::mem::replace(&mut own.0, attached);
            // Entries under a replaced node that weren't re-appended stayed
            // on the detached old node (e.g. mid-move to another parent) --
            // drop their stale back-pointers.
            for stale in old {
                if own.0.contains(&stale) {
                    continue;
                }
                if let Ok(mut dom_parent) = mirror.parents.get_mut(stale)
                    && dom_parent.0 == Some(entity)
                {
                    dom_parent.0 = None;
                }
            }
        }

        // Splice this fresh node into its parent unless the parent
        // will place it itself.
        let Ok(child_of) = parents.get(entity) else {
            continue;
        };
        let parent_entity = child_of.0;

        let Ok((parent_node, parent_children)) = nodes.get(parent_entity) else {
            continue;
        };

        if parent_node.is_changed() {
            continue;
        }

        let parent_children = match parent_children {
            Some(c) if !c.is_changed() => c,
            // The parent's `Children` changed -- `sync_child_order`
            // will handle this.
            _ => continue,
        };

        // The next sibling, in `Children` order, that is already attached
        // under the parent -- the anchor to insert before. Resolved from the
        // mirror (no `contains` DOM reads); because the mirror is updated in
        // lockstep, the result is independent of the order in which sibling
        // nodes are processed this tick.
        let parent_children: &[Entity] = parent_children.into_inner().as_ref();
        let next = parent_children
            .iter()
            .skip_while(|c| **c != entity)
            .skip(1)
            .copied()
            .find(|&sibling| mirror.is_attached_to(sibling, parent_entity));

        match next {
            Some(anchor) => {
                let (anchor_node, _) = nodes.get(anchor)?;
                parent_node
                    .insert_before(node, Some(anchor_node.as_ref()))
                    .js_err()?;
                mirror.attach(entity, parent_entity, Some(anchor))?;
            }
            None => {
                parent_node.append_child(node).js_err()?;
                mirror.attach(entity, parent_entity, None)?;
            }
        }
    }

    // A parent's `Children` changed but its `Node` did not -- reconcile
    // DOM order (and attach any children that weren't in the DOM yet).
    for (entity, node, children) in &changed_children {
        if node.is_changed() {
            continue;
        }
        sync_child_order(
            entity,
            node.into_inner(),
            children.as_ref(),
            &nodes,
            &mut mirror,
        )?;
    }

    Ok(())
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

/// Make the DOM order of `parent`'s entity-backed children match their
/// `Children` order, attaching any that aren't in the DOM yet. Nodes on a
/// longest increasing subsequence of the current order stay put, so the
/// number of `insert_before` calls (each a remove+insert that drops focus
/// and restarts animations on the moved node) is minimal. The current order
/// comes from the [`DomMirror`], so when the DOM already matches this does
/// no DOM work at all.
fn sync_child_order(
    parent_entity: Entity,
    parent: &html::Node,
    children: &[Entity],
    nodes: &Query<(Ref<html::Node>, Option<Ref<Children>>)>,
    mirror: &mut DomMirror,
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
        desired.push((*child, (**child_node).clone()));
    }

    // The current order, as desired-indices of the parent's attached entity
    // children — read from the mirror instead of walking `first_child`/
    // `next_sibling`. Attached entities no longer in `children` (and foreign
    // DOM nodes, which the mirror never contains) are skipped; moves are
    // anchored on managed nodes only, so they stay where they are.
    let current: Vec<usize> = mirror
        .children
        .get(parent_entity)?
        .0
        .iter()
        .filter_map(|child| desired_index.get(child).copied())
        .collect();

    let in_order = current.len() == desired.len() && current.is_sorted();
    if in_order {
        return Ok(());
    }

    // Mass attach (the patch-load path: N fresh children under one parent).
    // Nothing is kept, so a forward `append_child` walk lands them in order
    // with O(1) mirror pushes — the reverse `insert_before` walk below would
    // pay an O(len) mirror insert per child.
    if current.is_empty() {
        for (child, node) in &desired {
            parent.append_child(node).js_err()?;
            mirror.attach(*child, parent_entity, None)?;
        }
        return Ok(());
    }

    let keep: HashSet<usize> = longest_increasing_subsequence(&current)
        .into_iter()
        .collect();

    let mut anchor: Option<(Entity, web_sys::Node)> = None;
    for (i, (child, node)) in desired.iter().enumerate().rev() {
        if keep.contains(&i) {
            anchor = Some((*child, node.clone()));
            continue;
        }

        parent
            .insert_before(node, anchor.as_ref().map(|(_, node)| node))
            .js_err()?;
        mirror.attach(*child, parent_entity, anchor.as_ref().map(|(e, _)| *e))?;
        anchor = Some((*child, node.clone()));
    }

    Ok(())
}

/// The original DOM-walk implementation of [`sync_child_order`], kept only as
/// the behavioural oracle for the dead-code [`reparent`] reference above.
#[allow(dead_code)]
fn sync_child_order_dom_walk(
    parent: &html::Node,
    children: &[Entity],
    nodes: &Query<(Ref<html::Node>, Option<Ref<Children>>)>,
    lookup: &html::NodeLookup,
) -> Result {
    use bevy_platform::collections::{HashMap, HashSet};

    let mut desired = Vec::with_capacity(children.len());
    let mut desired_index = HashMap::with_capacity(children.len());
    for child in children {
        let Ok((child_node, _)) = nodes.get(*child) else {
            continue;
        };

        desired_index.insert(*child, desired.len());
        desired.push((**child_node).clone());
    }

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
