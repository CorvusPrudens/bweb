//! Effects: arbitrary systems, re-run whenever a signal they read changes.
//!
//! An effect is a derived signal with the value taken away. It discovers its
//! dependencies the same way — by reporting what it read through a
//! [`SignalStore`](crate::signal2::source::SignalStore) — is marked and
//! deduplicated the same way, and is torn down
//! the same way. What differs is the body: a whole system with its own
//! `SystemParam`s rather than a closure, and one under no read-only bound.
//!
//! That bound is why an effect cannot be dispatched from the scan the way a
//! mapper system is. The scan holds `&World`, which is enough for a read-only
//! mapper writing through a borrowed queue but not for a system with `ResMut`,
//! an `EventWriter`, or a `Commands` param of its own. So the passes only
//! *mark* effects, and the settle loop runs the marked ones in an exclusive
//! step once the derived pass has finished and flushed — by which point
//! everything this pass produced is in the world for the effect to see.

use bevy_ecs::{
    lifecycle::HookContext,
    prelude::*,
    system::{BoxedSystem, SystemId},
    world::DeferredWorld,
};

use crate::signal2::{
    ReactiveWork,
    derived::{DerivedNode, NodeLevel, NodeSources, resubscribe},
    source::SignalReads,
};

/// A registered effect body: takes the entity the effect was placed on.
pub type EffectSystem = SystemId<In<Entity>, ()>;

/// An arbitrary system, re-run whenever a signal it read changes.
///
/// Place it on an entity — `commands.spawn(Effect::new(my_system))`, or
/// `insert` it onto something that already exists. The system is handed that
/// entity as its input, so one body can serve many placements:
///
/// ```ignore
/// fn highlight(In(entity): In<Entity>, store: SignalStore, mut commands: Commands) {
///     let hovered = hovered_signal.get(&store)?;
///     commands.entity(entity).insert(Highlight(hovered.0));
/// }
///
/// commands.spawn((Button, Effect::new(highlight)));
/// ```
///
/// The effect stops when the component does: overwrite it, remove it, or
/// despawn the entity, and the subscriptions come off and the system is
/// unregistered. There is deliberately no `Clone` — the component *is* the
/// effect's lifetime, and a copy of it would be a second claim on something
/// only one of them can end.
///
/// # Placement
///
/// An effect keeps its graph state on the entity it is placed on, so it wants
/// an entity of its own or a plain one to hitch onto. The one combination to
/// avoid is placing an effect on a derived signal's entity
/// (`commands.entity(signal.entity()).insert(Effect::new(..))`), which
/// overwrites the node that signal is made of. Spawn it as a child instead.
#[derive(Component)]
#[component(
    on_insert = Effect::on_insert,
    on_replace = Effect::on_replace,
    clone_behavior = Ignore,
)]
pub struct Effect {
    /// Taken by `on_insert`, which hands it to the world's system registry.
    /// `None` from then on.
    system: Option<BoxedSystem<In<Entity>, ()>>,
    /// Where it ended up, so `on_replace` can unregister it.
    pub(crate) registered: Option<EffectSystem>,
}

impl Effect {
    #[must_use]
    pub fn new<S, M>(system: S) -> Self
    where
        S: IntoSystem<In<Entity>, (), M> + 'static,
        M: 'static,
    {
        Self {
            system: Some(Box::new(IntoSystem::into_system(system))),
            registered: None,
        }
    }

    fn on_insert(mut world: DeferredWorld, ctx: HookContext) {
        let entity = ctx.entity;
        let Some(mut effect) = world.get_mut::<Effect>(entity) else {
            return;
        };
        let Some(system) = effect.system.take() else {
            log::warn!(
                "signal2: the effect on {entity} was inserted twice; the second insert has \
                 been dropped"
            );
            return;
        };

        // Deferred because registering a system needs `&mut World`.
        world.commands().queue(move |world: &mut World| {
            // The entity can be despawned between the insert and this command,
            // in which case `on_replace` has already run and has nothing to
            // unregister — so don't register anything.
            if world.get_entity(entity).is_err() {
                return;
            }

            let registered = world.register_boxed_system(system);
            let Some(mut effect) = world.get_mut::<Effect>(entity) else {
                let _ = world.unregister_system(registered);
                return;
            };
            effect.registered = Some(registered);

            let mut node = world.entity_mut(entity);
            node.entry::<NodeLevel>().or_default();
            node.entry::<NodeSources>().or_default();
            // `DerivedNode::on_add` gives the effect its first run.
            node.insert(DerivedNode::effect());
        });
    }

    /// End the effect when the component goes away.
    ///
    /// `on_replace` is the one hook that fires for all three endings —
    /// overwritten, removed, despawned — so a re-insert also can't leave the old
    /// system registered and still subscribed.
    fn on_replace(mut world: DeferredWorld, ctx: HookContext) {
        let entity = ctx.entity;
        let Some(effect) = world.get::<Effect>(entity) else {
            return;
        };
        let registered = effect.registered;

        world.commands().queue(move |world: &mut World| {
            // Only reachable when the component alone went: on a despawn the
            // entity is already gone, and `DerivedNode::on_replace` did the
            // unwiring on the way out.
            if let Ok(mut node) = world.get_entity_mut(entity) {
                node.remove::<DerivedNode>();
            }

            // The system lives on an entity of its own, which no despawn of the
            // effect's entity will ever reach.
            if let Some(registered) = registered {
                let _ = world.unregister_system(registered);
            }
        });
    }
}

/// Whether `node` is an effect rather than a derived node.
///
/// Used to prevent a double-run on insertion for effect nodes.
pub(crate) fn reads_post_flush(world: &World, node: Entity) -> bool {
    world.get::<Effect>(node).is_some()
}

/// Run every effect marked during this pass.
///
/// Exclusive, and outside the pass's `resource_scope`: an effect is an ordinary
/// system that may touch anything, up to and including spawning more reactive
/// state. `run_system_with` applies what each one queues before the next starts,
/// so an effect reading what an earlier effect wrote sees it.
pub(crate) fn run_pending_effects(world: &mut World, work: &mut ReactiveWork) {
    for node in work.take_effects() {
        let Some(system) = world
            .get::<Effect>(node)
            .and_then(|effect| effect.registered)
        else {
            // Removed between being marked and being run.
            continue;
        };

        // Cleared immediately before and harvested immediately after: an
        // effect's dependencies are whatever it read on this run, exactly as for
        // a derived node.
        world.resource::<SignalReads>().clear();
        let result = world.run_system_with(system, node);
        let reads = world.resource::<SignalReads>().take();

        if let Err(e) = result {
            log::error!("signal2: the effect on {node} failed to run: {e}");
            // Deliberately no resubscribe. A system that never ran reported no
            // reads, and taking that at face value would unsubscribe the effect
            // from everything and leave it dead for good — where keeping the old
            // edges gets it woken again on the next change.
            continue;
        }

        resubscribe(world, node, &reads);
    }
}
