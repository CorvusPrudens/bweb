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
