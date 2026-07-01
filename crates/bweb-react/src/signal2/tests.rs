use super::*;
use crate::cleanup::CleanupPlugin;
use crate::signal2::graph::{PendingDirty, SignalSystem};
use bevy_ecs::prelude::*;
use bevy_query_observer::Start;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Simulates an input firing: overwrite its cached value, then enqueue it as
/// a changed source so the next flush marks its subscribers.
fn drive_input<O: Send + Sync + 'static>(app: &mut App, input: &DerivedSignal<O>, value: O) {
    *input.value.write().unwrap() = Some(value);
    app.world_mut()
        .resource_mut::<PendingDirty>()
        .0
        .push(input.inner.entity);
}

fn signal_node_count(world: &mut World) -> usize {
    let mut q = world.query::<&SignalSystem>();
    q.iter(world).count()
}

/// `xy → {x, y} → area`. When `xy` changes, `area` must recompute exactly
/// once (not once per changed arm) and see a consistent view of both.
#[test]
fn diamond_settles_shared_node_once() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let x_runs = Arc::new(AtomicUsize::new(0));
    let area_runs = Arc::new(AtomicUsize::new(0));

    let world = app.world_mut();
    let mut commands = world.commands();

    // Apex driven manually via `drive_input` (stands in for an observer input).
    let xy = commands.derive(|| Ok(2.0_f32));
    let x = {
        let xy = xy.clone();
        let runs = x_runs.clone();
        commands.derive(move || {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(xy.get()?)
        })
    };
    let y = {
        let xy = xy.clone();
        commands.derive(move || Ok(xy.get()?))
    };
    let area = {
        let (x, y, runs) = (x.clone(), y.clone(), area_runs.clone());
        commands.derive(move || {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(x.get()? * y.get()?)
        })
    };

    app.update();

    // Initial evaluation: everything ran once, values flowed through.
    assert_eq!(x_runs.load(Ordering::Relaxed), 1);
    assert_eq!(area_runs.load(Ordering::Relaxed), 1);
    assert_eq!(area.get(), Ok(4.0));

    // Change the apex and flush.
    drive_input(&mut app, &xy, 3.0);
    app.update();

    // Both arms re-ran, but the shared node recomputed only once more.
    assert_eq!(x_runs.load(Ordering::Relaxed), 2);
    assert_eq!(area_runs.load(Ordering::Relaxed), 2);
    assert_eq!(area.get(), Ok(9.0));

    // A flush with nothing pending is a no-op — no idle recomputation.
    app.update();
    assert_eq!(area_runs.load(Ordering::Relaxed), 2);
}

/// A node outside the changed subgraph is never recomputed.
#[test]
fn unrelated_node_is_not_recomputed() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let dependent_runs = Arc::new(AtomicUsize::new(0));
    let unrelated_runs = Arc::new(AtomicUsize::new(0));

    let world = app.world_mut();
    let mut commands = world.commands();

    let source = commands.derive(|| Ok(1.0_f32));
    let _dependent = {
        let source = source.clone();
        let runs = dependent_runs.clone();
        commands.derive(move || {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(source.get()? + 1.0)
        })
    };
    let _unrelated = {
        let runs = unrelated_runs.clone();
        commands.derive(move || {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(42.0_f32)
        })
    };

    app.update();
    assert_eq!(dependent_runs.load(Ordering::Relaxed), 1);
    assert_eq!(unrelated_runs.load(Ordering::Relaxed), 1);

    drive_input(&mut app, &source, 5.0);
    app.update();

    // Only the dependent chain re-ran.
    assert_eq!(dependent_runs.load(Ordering::Relaxed), 2);
    assert_eq!(unrelated_runs.load(Ordering::Relaxed), 1);
}

/// `NotReady` from a source propagates through `?`, and clears once the
/// source becomes ready.
#[test]
fn not_ready_propagates() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let world = app.world_mut();
    let mut commands = world.commands();

    let input = commands.derive(|| Ok(0.0_f32));
    let gated = {
        let input = input.clone();
        commands.derive(move || {
            let v = input.get()?;
            if v > 0.0 {
                Ok(v)
            } else {
                Err(SignalError::NotReady)
            }
        })
    };
    let dependent = {
        let gated = gated.clone();
        commands.derive(move || Ok(gated.get()? + 1.0))
    };

    app.update();
    assert_eq!(gated.get(), Err(SignalError::NotReady));
    assert_eq!(dependent.get(), Err(SignalError::NotReady));

    drive_input(&mut app, &input, 7.0);
    app.update();
    assert_eq!(gated.get(), Ok(7.0));
    assert_eq!(dependent.get(), Ok(8.0));
}

#[derive(Component, Clone, PartialEq, Debug)]
struct Tag(u32);

#[derive(Component, Clone, PartialEq, Debug)]
struct Doubled(u32);

/// A `DerivedSignal<Bundle>` dropped on an entity reactively (re)inserts its
/// value, and the sink is torn down when the host is despawned.
#[test]
fn reactive_bundle_insertion_and_teardown() {
    let mut app = App::new();
    app.add_plugins((Signal2Plugin, CleanupPlugin));

    let world = app.world_mut();
    let mut commands = world.commands();

    let input = commands.derive(|| Ok(1u32));
    let tag = {
        let input = input.clone();
        commands.derive(move || Ok(Tag(input.get()?)))
    };
    let host = commands.spawn(tag.clone()).id();

    app.update();
    assert_eq!(app.world().get::<Tag>(host), Some(&Tag(1)));

    drive_input(&mut app, &input, 5);
    app.update();
    assert_eq!(app.world().get::<Tag>(host), Some(&Tag(5)));

    // Teardown: despawning the host removes its sink node.
    let before = signal_node_count(app.world_mut());
    app.world_mut().despawn(host);
    app.update();
    assert_eq!(signal_node_count(app.world_mut()), before - 1);
}

/// `.map(|&v| ...)` inserts (and keeps current) a bundle mapped by reference.
#[test]
fn map_by_reference_inserts() {
    let mut app = App::new();
    app.add_plugins((Signal2Plugin, CleanupPlugin));

    let world = app.world_mut();
    let mut commands = world.commands();

    let input = commands.derive(|| Ok(2u32));
    let host = commands.spawn(input.map(|v: &u32| Doubled(v * 2))).id();

    app.update();
    assert_eq!(app.world().get::<Doubled>(host), Some(&Doubled(4)));

    drive_input(&mut app, &input, 10);
    app.update();
    assert_eq!(app.world().get::<Doubled>(host), Some(&Doubled(20)));
}

/// `.option()` inserts the inner bundle on `Some` and removes it on `None`.
#[test]
fn option_removes_on_none() {
    let mut app = App::new();
    app.add_plugins((Signal2Plugin, CleanupPlugin));

    let world = app.world_mut();
    let mut commands = world.commands();

    let input = commands.derive(|| Ok(1u32));
    let maybe = {
        let input = input.clone();
        commands.derive(move || {
            let v = input.get()?;
            Ok((v > 0).then_some(Tag(v)))
        })
    };
    let host = commands.spawn(maybe.option()).id();

    app.update();
    assert_eq!(app.world().get::<Tag>(host), Some(&Tag(1)));

    drive_input(&mut app, &input, 0);
    app.update();
    assert_eq!(app.world().get::<Tag>(host), None);
}

/// A memo whose recompute yields an equal value does not re-run its
/// subscribers; a value that actually moves does.
#[test]
fn memo_prunes_unchanged() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let downstream_runs = Arc::new(AtomicUsize::new(0));

    let world = app.world_mut();
    let mut commands = world.commands();

    let input = commands.derive(|| Ok(2u32));
    let parity = {
        let input = input.clone();
        commands.memo(move || Ok(input.get()? % 2))
    };
    let downstream = {
        let parity = parity.clone();
        let runs = downstream_runs.clone();
        commands.derive(move || {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(parity.get()?)
        })
    };

    app.update();
    assert_eq!(downstream_runs.load(Ordering::Relaxed), 1);
    assert_eq!(downstream.get(), Ok(0));

    // 2 -> 4: parity stays 0, so the memo prunes — downstream must not re-run.
    drive_input(&mut app, &input, 4);
    app.update();
    assert_eq!(downstream_runs.load(Ordering::Relaxed), 1);
    assert_eq!(downstream.get(), Ok(0));

    // 4 -> 5: parity flips 0 -> 1, so the change propagates.
    drive_input(&mut app, &input, 5);
    app.update();
    assert_eq!(downstream_runs.load(Ordering::Relaxed), 2);
    assert_eq!(downstream.get(), Ok(1));
}

#[derive(Resource, Clone)]
struct External(u32);

/// A `poll` node re-runs every flush (tracking non-observable world state), but
/// only propagates when its value actually moves.
#[test]
fn poll_runs_every_flush_and_prunes() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);
    app.insert_resource(External(1));

    let poll_runs = Arc::new(AtomicUsize::new(0));
    let downstream_runs = Arc::new(AtomicUsize::new(0));

    let world = app.world_mut();
    let mut commands = world.commands();

    let polled = {
        let runs = poll_runs.clone();
        commands.poll(move |ext: Res<External>| {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(ext.0)
        })
    };
    let downstream = {
        let polled = polled.clone();
        let runs = downstream_runs.clone();
        commands.derive(move || {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(polled.get()?)
        })
    };

    app.update();
    assert_eq!(downstream.get(), Ok(1));
    let poll_after_first = poll_runs.load(Ordering::Relaxed);
    let downstream_after_first = downstream_runs.load(Ordering::Relaxed);
    assert!(poll_after_first >= 1);

    // No external change: the poll re-runs but prunes — downstream stays put.
    app.update();
    assert!(poll_runs.load(Ordering::Relaxed) > poll_after_first);
    assert_eq!(downstream_runs.load(Ordering::Relaxed), downstream_after_first);

    // The resource changes with no lifecycle event; the poll still catches it.
    app.world_mut().resource_mut::<External>().0 = 9;
    app.update();
    assert_eq!(downstream.get(), Ok(9));
    assert_eq!(
        downstream_runs.load(Ordering::Relaxed),
        downstream_after_first + 1
    );
}

#[derive(Component, Clone)]
struct Count(u32);

/// `watch_entity` wires the query observer to a specific entity: changes to
/// that entity drive the signal.
#[test]
fn watch_entity_drives_signal() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let watched = app.world_mut().spawn(Count(3)).id();

    let count = {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands
            .signal(|c: Start<&Count>| c.0)
            .watch_entity(watched)
    };

    // Observer is built + seeded during finalization.
    app.update();
    assert_eq!(count.get(), Ok(3));

    // A later change to the watched entity drives the signal.
    app.world_mut().entity_mut(watched).insert(Count(7));
    app.update();
    assert_eq!(count.get(), Ok(7));
}

/// `watch_bundle` wires the query observer to the entity the signal is
/// inserted into as a bundle.
#[test]
fn watch_bundle_watches_host() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let count = {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands.signal(|c: Start<&Count>| c.0)
    };

    let host = {
        let watch = count.watch_bundle();
        app.world_mut().spawn((Count(9), watch)).id()
    };

    app.update();
    assert_eq!(count.get(), Ok(9));

    app.world_mut().entity_mut(host).insert(Count(11));
    app.update();
    assert_eq!(count.get(), Ok(11));
}

/// Snapshots a parent's children as an owned `Vec` — the extractor most reactive
/// lists want (absent `Children` reads as an empty list).
fn child_list(c: Option<&Children>) -> Vec<Entity> {
    c.map_or_else(Vec::new, |c| c.iter().collect::<Vec<Entity>>())
}

/// `track` sees in-place `Children` mutations that no query observer fires on:
/// appending a child to a non-empty parent, and reordering.
#[test]
fn track_children_sees_add_and_reorder() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let parent = app.world_mut().spawn_empty().id();
    let c1 = app.world_mut().spawn(ChildOf(parent)).id();

    let children = {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands.track(child_list).watch_entity(parent)
    };

    app.update();
    assert_eq!(children.get(), Ok(vec![c1]));

    // Parent already has `Children`, so appending mutates it in place — no
    // lifecycle event, invisible to an observer, caught by the scanner.
    let c2 = app.world_mut().spawn(ChildOf(parent)).id();
    app.update();
    assert_eq!(children.get(), Ok(vec![c1, c2]));

    // Reorder (move c1 to the end): also invisible to a `ChildOf` observer.
    app.world_mut().entity_mut(parent).remove_children(&[c1]);
    app.world_mut().entity_mut(parent).add_children(&[c1]);
    app.update();
    assert_eq!(children.get(), Ok(vec![c2, c1]));
}

/// When the last child leaves, bevy removes the `Children` component entirely —
/// which `Changed<Children>` can't see; the `On<Remove, Children>` observer does.
#[test]
fn track_children_sees_empty_on_removal() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let parent = app.world_mut().spawn_empty().id();
    let c1 = app.world_mut().spawn(ChildOf(parent)).id();

    let children = {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands.track(child_list).watch_entity(parent)
    };

    app.update();
    assert_eq!(children.get(), Ok(vec![c1]));

    app.world_mut().entity_mut(c1).remove::<ChildOf>();
    app.update();
    assert_eq!(children.get(), Ok(vec![]));
}

/// A mutation is reflected through a downstream `derive` after a single update:
/// the scanner (`Scan`) runs before the settle (`Settle`) in the same schedule.
#[test]
fn track_settles_same_frame() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let parent = app.world_mut().spawn_empty().id();
    app.world_mut().spawn(ChildOf(parent));

    let doubled = {
        let world = app.world_mut();
        let mut commands = world.commands();
        let count = commands
            .track(|c: Option<&Children>| c.map_or(0usize, |c| c.iter().count()))
            .watch_entity(parent);
        commands.derive(move || Ok(count.get()? * 2))
    };

    app.update();
    assert_eq!(doubled.get(), Ok(2));

    app.world_mut().spawn(ChildOf(parent));
    app.update();
    assert_eq!(doubled.get(), Ok(4));
}

/// With no `Children` change, the scanner still runs but pushes nothing, so the
/// downstream node is never recomputed — idle cost is scan-only.
#[test]
fn track_idle_prunes() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let parent = app.world_mut().spawn_empty().id();
    app.world_mut().spawn(ChildOf(parent));

    let runs = Arc::new(AtomicUsize::new(0));
    let _downstream = {
        let world = app.world_mut();
        let mut commands = world.commands();
        let children = commands.track(child_list).watch_entity(parent);
        let runs = runs.clone();
        commands.derive(move || {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(children.get()?.len())
        })
    };

    // Let bind/seed and any first-run scan settle, then take a baseline.
    app.update();
    app.update();
    let baseline = runs.load(Ordering::Relaxed);

    app.update();
    app.update();
    assert_eq!(runs.load(Ordering::Relaxed), baseline);
}

/// The first `track` over a component type registers its machinery; further
/// tracks of the same type reuse it, and a new type registers its own.
#[test]
fn track_on_demand_registers_once() {
    use super::track::TrackedTypes;

    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let e = app.world_mut().spawn_empty().id();
    {
        let world = app.world_mut();
        let mut commands = world.commands();
        // Two trackers of `Children` (different output types, same component).
        let _a = commands.track(|c: Option<&Children>| c.is_some()).watch_entity(e);
        let _b = commands
            .track(|c: Option<&Children>| c.map_or(0usize, |c| c.iter().count()))
            .watch_entity(e);
    }
    app.update();
    assert_eq!(app.world().resource::<TrackedTypes>().0.len(), 1);

    {
        let world = app.world_mut();
        let mut commands = world.commands();
        let _c = commands.track(|t: Option<&Tag>| t.map(|t| t.0)).watch_entity(e);
    }
    app.update();
    assert_eq!(app.world().resource::<TrackedTypes>().0.len(), 2);
}

/// A tracker keyed on one entity is undisturbed by another entity's `Children`
/// changing — the scanner routes by entity through the registry.
#[test]
fn track_unrelated_entity() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);

    let watched = app.world_mut().spawn_empty().id();
    app.world_mut().spawn(ChildOf(watched));
    let other = app.world_mut().spawn_empty().id();

    let runs = Arc::new(AtomicUsize::new(0));
    let children = {
        let world = app.world_mut();
        let mut commands = world.commands();
        let children = commands
            .track(|c: Option<&Children>| c.map_or(0usize, |c| c.iter().count()))
            .watch_entity(watched);
        let runs = runs.clone();
        let downstream_src = children.clone();
        commands.derive(move || {
            runs.fetch_add(1, Ordering::Relaxed);
            downstream_src.get()
        });
        children
    };

    app.update();
    app.update();
    let baseline = runs.load(Ordering::Relaxed);

    // Mutate a different entity's `Children`.
    app.world_mut().spawn(ChildOf(other));
    app.update();
    assert_eq!(runs.load(Ordering::Relaxed), baseline);
    assert_eq!(children.get(), Ok(1));
}

// ---------------------------------------------------------------------------
// Garbage collection (Arc-strong-count)
// ---------------------------------------------------------------------------

use super::gc::SignalGc;

/// Number of live readable nodes (each carries a [`SignalGc`] probe).
fn gc_node_count(world: &mut World) -> usize {
    let mut q = world.query::<&SignalGc>();
    q.iter(world).count()
}

/// Total number of live entities.
fn entity_count(world: &mut World) -> usize {
    let mut q = world.query::<Entity>();
    q.iter(world).count()
}

/// A derive whose last handle is dropped falls to its rest count and is collected
/// on the next sweep.
#[test]
fn gc_collects_dropped_derive() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);
    app.insert_resource(SweepFrequency(core::time::Duration::ZERO));

    let handle = {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands.derive(|| Ok(1u32))
    };

    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 1);

    drop(handle);
    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 0);
}

/// A derive whose handle stays alive is never collected, sweep after sweep.
#[test]
fn gc_keeps_referenced_derive() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);
    app.insert_resource(SweepFrequency(core::time::Duration::ZERO));

    let _handle = {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands.derive(|| Ok(1u32))
    };

    for _ in 0..4 {
        app.update();
        assert_eq!(gc_node_count(app.world_mut()), 1);
    }
}

/// A subscriber pins its source through the captured handle: `a` survives while `b`
/// (which reads it) lives, and the abandoned chain collects leaf-first.
#[test]
fn gc_cascades() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);
    app.insert_resource(SweepFrequency(core::time::Duration::ZERO));

    let a = {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands.derive(|| Ok(1u32))
    };
    let b = {
        let world = app.world_mut();
        let mut commands = world.commands();
        let a = a.clone();
        commands.derive(move || Ok(a.get()? + 1))
    };

    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 2);

    // Drop `a`'s handle: `a` is still pinned by `b`'s captured clone.
    drop(a);
    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 2);

    // Drop `b`: `b` collects this sweep, releasing its capture of `a`; `a` then
    // collects on the following sweep (leaf-first cascade).
    drop(b);
    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 1);
    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 0);
}

/// Collecting a `track` node also purges its off-node `TrackedSources<T>` entry via
/// the `TrackGc` `on_remove` hook.
#[test]
fn gc_collects_track_and_purges_registry() {
    use super::track::TrackedSources;

    let mut app = App::new();
    app.add_plugins(Signal2Plugin);
    app.insert_resource(SweepFrequency(core::time::Duration::ZERO));

    let parent = app.world_mut().spawn_empty().id();
    app.world_mut().spawn(ChildOf(parent));

    let children = {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands.track(child_list).watch_entity(parent)
    };

    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 1);
    assert!(
        app.world()
            .resource::<TrackedSources<Children>>()
            .0
            .contains_key(&parent)
    );

    drop(children);
    app.update();
    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 0);
    assert!(
        !app.world()
            .resource::<TrackedSources<Children>>()
            .0
            .contains_key(&parent)
    );
}

/// Collecting a `poll` node unregisters its system (fixing the leak): the poll system
/// stops running once the node is gone.
#[test]
fn gc_unregisters_poll_system() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);
    app.insert_resource(SweepFrequency(core::time::Duration::ZERO));
    app.insert_resource(External(1));

    let runs = Arc::new(AtomicUsize::new(0));
    let polled = {
        let world = app.world_mut();
        let mut commands = world.commands();
        let runs = runs.clone();
        commands.poll(move |ext: Res<External>| {
            runs.fetch_add(1, Ordering::Relaxed);
            Ok(ext.0)
        })
    };

    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 1);

    // Drop the handle and let the node collect (which unregisters the system).
    drop(polled);
    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 0);

    // The poll system no longer runs on subsequent flushes.
    let after = runs.load(Ordering::Relaxed);
    app.update();
    app.update();
    assert_eq!(runs.load(Ordering::Relaxed), after);
}

/// Within the grace interval, a dropped handle's node is *not* collected — the sweep
/// only fires once per `SweepFrequency`.
#[test]
fn gc_respects_grace() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin); // default SweepFrequency (well above a test's wall-clock)

    let handle = {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands.derive(|| Ok(1u32))
    };

    app.update();
    assert_eq!(gc_node_count(app.world_mut()), 1);

    drop(handle);
    for _ in 0..4 {
        app.update();
    }
    // Grace interval has not elapsed, so the node survives despite being at rest.
    assert_eq!(gc_node_count(app.world_mut()), 1);
}

/// Collecting a `signal` node leaves no orphan query-observer entity: repeated
/// create-then-collect cycles don't grow the live entity count.
#[test]
fn gc_no_orphan_observer() {
    let mut app = App::new();
    app.add_plugins(Signal2Plugin);
    app.insert_resource(SweepFrequency(core::time::Duration::ZERO));

    let watched = app.world_mut().spawn(Count(3)).id();

    let mut counts = Vec::new();
    for _ in 0..3 {
        {
            let world = app.world_mut();
            let mut commands = world.commands();
            // Handle dropped at the end of this block; the node builds then collects.
            let _sig = commands.signal(|c: Start<&Count>| c.0).watch_entity(watched);
        }
        app.update();
        app.update();
        app.update();
        assert_eq!(gc_node_count(app.world_mut()), 0);
        counts.push(entity_count(app.world_mut()));
    }

    // One-time observer infrastructure lands in cycle 0; no per-signal leak means the
    // live entity count is identical across later cycles.
    assert_eq!(counts[0], counts[1]);
    assert_eq!(counts[1], counts[2]);
}
