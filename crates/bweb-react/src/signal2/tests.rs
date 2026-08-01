use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use bevy_app::App;
use bevy_ecs::prelude::*;

use crate::signal2::{
    ReactivePlugin, SignalData, SignalExt, SubscriberSet,
    derived::{NodeLevel, NodeSources},
    run_reactive_schedule,
    source::{SignalStore, SourceSignal},
};

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Source(i32);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Other(i32);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Doubled(i32);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Quadrupled(i32);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Sum(i32);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Left(i32);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Right(i32);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Bottom(i32);

fn double(source: &Source) -> Doubled {
    Doubled(source.0 * 2)
}

fn quadruple(doubled: &Doubled) -> Quadrupled {
    Quadrupled(doubled.0 * 2)
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(ReactivePlugin);
    app
}

/// How many live subscriptions `entity` carries for `D`.
fn subscribers<D: SignalData>(world: &World, entity: Entity) -> usize
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    world
        .get::<SubscriberSet<D>>(entity)
        .map_or(0, |set| set.len())
}

fn level<D: SignalData>(world: &World, signal: &SourceSignal<D>) -> u16
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    world
        .get::<NodeLevel>(signal.entity())
        .map_or(u16::MAX, |level| level.0)
}

/// The scan only visits entities whose data changed, so a subscriber that
/// attaches to a quiet source would otherwise never run at all.
#[test]
fn maps_on_insertion_without_a_change() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(21)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map(double)).id()
    };
    world.flush();

    // No schedule run: the value must already be there.
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(42)));
}

#[test]
fn maps_again_when_the_source_changes() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map(double)).id()
    };
    world.flush();
    run_reactive_schedule(world);

    world.get_mut::<Source>(source).unwrap().0 = 5;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(10)));
}

/// Each mapped hop writes through `Commands`, so the second hop can only see
/// the first one's output on a later pass. The settle loop is what keeps that
/// from costing a frame per level.
#[test]
fn a_two_hop_chain_settles_in_one_frame() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(3)).id();

    let (first, second) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let first = commands.spawn(signal.map(double)).id();

        let doubled = commands.signal::<&Doubled>().watch(first);
        let second = commands.spawn(doubled.map(quadruple)).id();
        (first, second)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(first), Some(&Doubled(6)));
    assert_eq!(world.get::<Quadrupled>(second), Some(&Quadrupled(12)));

    world.get_mut::<Source>(source).unwrap().0 = 10;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Quadrupled>(second), Some(&Quadrupled(40)));
}

/// Two inputs changing in the same frame must produce one evaluation, not one
/// per input — that is the whole reason the scan marks derived nodes instead of
/// dispatching them.
#[test]
fn a_derived_signal_runs_once_for_two_changed_inputs() {
    let mut app = app();
    let world = app.world_mut();
    let a_entity = world.spawn(Source(1)).id();
    let b_entity = world.spawn(Other(10)).id();

    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();

    let sum = {
        let mut commands = world.commands();
        let a = commands.signal::<&Source>().watch(a_entity);
        let b = commands.signal::<&Other>().watch(b_entity);

        commands.derive(move |s| {
            counter.fetch_add(1, Ordering::Relaxed);
            let a = a.get(s)?.0;
            let b = b.get(s)?.0;
            Ok(Sum(a + b))
        })
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(sum.entity()), Some(&Sum(11)));

    let settled = runs.load(Ordering::Relaxed);
    world.get_mut::<Source>(a_entity).unwrap().0 = 2;
    world.get_mut::<Other>(b_entity).unwrap().0 = 20;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(sum.entity()), Some(&Sum(22)));
    assert_eq!(
        runs.load(Ordering::Relaxed) - settled,
        1,
        "both inputs changed in one frame, so the node should evaluate once"
    );
}

/// A diamond: one source feeds two derived nodes that both feed a third. The
/// bottom node must not run on a half-updated pair.
#[test]
fn a_diamond_settles_once_and_consistently() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(4)).id();

    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();

    let bottom = {
        let mut commands = world.commands();
        let a = commands.signal::<&Source>().watch(source);

        let left_source = a.clone();
        let left = commands.derive(move |s| Ok(Left(left_source.get(s)?.0 * 2)));

        let right = commands.derive(move |s| Ok(Right(a.get(s)?.0 + 1)));

        commands.derive(move |s| {
            counter.fetch_add(1, Ordering::Relaxed);
            let left = left.get(s)?.0;
            let right = right.get(s)?.0;
            Ok(Bottom(left + right))
        })
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Bottom>(bottom.entity()), Some(&Bottom(8 + 5)));

    let settled = runs.load(Ordering::Relaxed);
    world.get_mut::<Source>(source).unwrap().0 = 10;
    run_reactive_schedule(world);

    assert_eq!(
        world.get::<Bottom>(bottom.entity()),
        Some(&Bottom(20 + 11)),
        "the bottom node saw a consistent pair"
    );
    assert_eq!(
        runs.load(Ordering::Relaxed) - settled,
        1,
        "both branches updated in the same pass, so the join runs once"
    );
}

/// Levels come from what a node actually read, so they are only correct once a
/// node has evaluated at least once.
#[test]
fn levels_follow_the_dependency_depth() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let (first, second) = {
        let mut commands = world.commands();
        let a = commands.signal::<&Source>().watch(source);
        let first = commands.derive(move |s| Ok(Left(a.get(s)?.0)));

        let upstream = first.clone();
        let second = commands.derive(move |s| Ok(Right(upstream.get(s)?.0)));
        (first, second)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<NodeLevel>(first.entity()), Some(&NodeLevel(1)));
    assert_eq!(world.get::<NodeLevel>(second.entity()), Some(&NodeLevel(2)));
}

/// A level is discovered, not declared, so a node can gain a deeper input long
/// after its own subscribers were wired. If the raise stopped at the node, those
/// subscribers would claim to be shallower than their input and cost a settle
/// pass every time the two went dirty together.
#[test]
fn a_level_raise_reaches_the_nodes_downstream() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let read_deep = Arc::new(AtomicBool::new(false));
    let branch = read_deep.clone();

    let (deep, mid, bottom) = {
        let mut commands = world.commands();
        let a = commands.signal::<&Source>().watch(source);

        let shallow = a.clone();
        let first = commands.derive(move |s| Ok(Left(shallow.get(s)?.0)));

        let upstream = first.clone();
        let deep = commands.derive(move |s| Ok(Right(upstream.get(s)?.0)));

        // Reads the level-0 source at first, and the level-2 node after the flip.
        let deeper = deep.clone();
        let mid = commands.derive(move |s| {
            let value = if branch.load(Ordering::Relaxed) {
                deeper.get(s)?.0
            } else {
                a.get(s)?.0
            };

            Ok(Quadrupled(value))
        });

        let above = mid.clone();
        let bottom = commands.derive(move |s| Ok(Bottom(above.get(s)?.0)));

        (deep, mid, bottom)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(level(world, &deep), 2);
    assert_eq!(level(world, &mid), 1);
    assert_eq!(level(world, &bottom), 2);

    read_deep.store(true, Ordering::Relaxed);
    world.get_mut::<Source>(source).unwrap().0 = 7;
    run_reactive_schedule(world);

    assert_eq!(level(world, &mid), 3, "mid now reads a level-2 node");
    assert_eq!(
        level(world, &bottom),
        4,
        "and the raise carried on to mid's own subscriber"
    );
    assert_eq!(world.get::<Bottom>(bottom.entity()), Some(&Bottom(7)));
}

/// Lazy tracking cuts both ways: a branch that stops reading an input has to
/// stop being woken by it, or every conditional node degrades into one that
/// re-runs on the union of every input it has ever touched.
#[test]
fn a_derived_node_drops_the_edges_it_stops_reading() {
    let mut app = app();
    let world = app.world_mut();
    let left_entity = world.spawn(Left(1)).id();
    let right_entity = world.spawn(Right(100)).id();

    let read_left = Arc::new(AtomicBool::new(true));
    let branch = read_left.clone();
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();

    let node = {
        let mut commands = world.commands();
        let left = commands.signal::<&Left>().watch(left_entity);
        let right = commands.signal::<&Right>().watch(right_entity);

        commands.derive(move |s| {
            counter.fetch_add(1, Ordering::Relaxed);
            let value = if branch.load(Ordering::Relaxed) {
                left.get(s)?.0
            } else {
                right.get(s)?.0
            };

            Ok(Sum(value))
        })
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(1)));
    assert_eq!(
        world.get::<NodeSources>(node.entity()).unwrap().0.len(),
        1,
        "only the branch it took is a dependency"
    );

    // Flip the branch, and poke the input it is still reading to wake it.
    read_left.store(false, Ordering::Relaxed);
    world.get_mut::<Left>(left_entity).unwrap().0 = 2;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(100)));
    assert_eq!(subscribers::<&Left>(world, left_entity), 0);
    assert_eq!(subscribers::<&Right>(world, right_entity), 1);

    let settled = runs.load(Ordering::Relaxed);
    world.get_mut::<Left>(left_entity).unwrap().0 = 3;
    run_reactive_schedule(world);
    assert_eq!(
        runs.load(Ordering::Relaxed),
        settled,
        "the abandoned input must no longer wake the node"
    );

    world.get_mut::<Right>(right_entity).unwrap().0 = 200;
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(200)));
}

#[test]
fn a_despawned_mapped_signal_unsubscribes() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map(double)).id()
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(subscribers::<&Source>(world, source), 1);

    world.despawn(target);
    world.flush();
    assert_eq!(subscribers::<&Source>(world, source), 0);

    // And the source must still be usable afterwards.
    world.get_mut::<Source>(source).unwrap().0 = 5;
    run_reactive_schedule(world);
}

/// The old subscription has to go before the new one lands, or the source ends
/// up with two closures writing the same component on every change.
#[test]
fn re_inserting_a_mapped_signal_does_not_double_subscribe() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let (signal, target) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let target = commands.spawn(signal.clone().map(double)).id();
        (signal, target)
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(subscribers::<&Source>(world, source), 1);

    world.entity_mut(target).insert(signal.map(double));
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(subscribers::<&Source>(world, source), 1);
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(2)));
}

#[test]
fn a_despawned_derived_node_unsubscribes() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let node = {
        let mut commands = world.commands();
        let a = commands.signal::<&Source>().watch(source);
        commands.derive(move |s| Ok(Sum(a.get(s)?.0)))
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(subscribers::<&Source>(world, source), 1);

    world.despawn(node.entity());
    world.flush();
    assert_eq!(subscribers::<&Source>(world, source), 0);

    world.get_mut::<Source>(source).unwrap().0 = 9;
    run_reactive_schedule(world);
}

/// A system registered for a type that already has live entities starts life
/// with a backdated `last_run`, which would replay the entire existing
/// population through subscribers that were just brought up to date by hand.
#[test]
fn a_newly_registered_system_does_not_replay_history() {
    static CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counting_double(source: &Source) -> Doubled {
        CALLS.fetch_add(1, Ordering::Relaxed);
        Doubled(source.0 * 2)
    }

    let mut app = app();
    let world = app.world_mut();

    // Changed several frames before anything reactive exists.
    let source = world.spawn(Source(21)).id();
    world.flush();
    run_reactive_schedule(world);

    CALLS.store(0, Ordering::Relaxed);
    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map(counting_double)).id()
    };
    world.flush();

    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        1,
        "attaching evaluates exactly once"
    );

    run_reactive_schedule(world);
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        1,
        "the scan's first run must not re-dispatch the source's history"
    );
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(42)));
}
