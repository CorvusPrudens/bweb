use std::cell::RefCell;

use bevy_ecs::entity::{Entity, EntityIndexSet};

thread_local! {
    static COLLECTOR: RefCell<Option<EntityIndexSet>> = const { RefCell::new(None) };
}

/// Tracks which source signals the currently-evaluating node reads.
///
/// While a node's system runs inside [`ReactiveContext::collect`], every signal
/// read calls [`ReactiveContext::register`] with the source node's entity. The
/// collected set becomes the node's dependency (source) edges.
pub struct ReactiveContext;

impl ReactiveContext {
    /// Runs `f`, returning its result alongside the set of source entities read
    /// during it. Nesting is supported: the previous collector is restored on
    /// exit, so a node evaluated inside another node doesn't steal its reads.
    pub fn collect<F, O>(f: F) -> (O, EntityIndexSet)
    where
        F: FnOnce() -> O,
    {
        let prev = COLLECTOR.with_borrow_mut(|c| c.replace(EntityIndexSet::new()));
        let result = f();
        let collected = COLLECTOR
            .with_borrow_mut(|c| core::mem::replace(c, prev))
            .unwrap_or_default();
        (result, collected)
    }

    /// Registers that the currently-evaluating node read `source`.
    ///
    /// A no-op when called outside [`collect`] (e.g. a plain read from a system
    /// that isn't itself a reactive node).
    pub fn register(source: Entity) {
        COLLECTOR.with_borrow_mut(|c| {
            if let Some(sources) = c.as_mut() {
                sources.insert(source);
            }
        });
    }
}
