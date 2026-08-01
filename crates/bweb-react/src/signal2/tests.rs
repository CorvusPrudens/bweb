use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use bevy_app::App;
use bevy_ecs::prelude::*;

use crate::signal2::{
    ReactivePlugin, RegisteredSignalSystem, SignalCtx, SignalData, SignalExt, SubscriberSet,
    derived::{NodeLevel, NodeSources},
    run_reactive_schedule, settle_reactive,
    source::SourceSignal,
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

// ---------------------------------------------------------------------------
// Mapper systems
// ---------------------------------------------------------------------------

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Echo(i32);

#[derive(Resource, Clone, Copy)]
struct Sink(Entity);

#[derive(Resource, Clone, Copy)]
struct Scale(i32);

fn double_system(ctx: SignalCtx<&Source>) -> Doubled {
    Doubled(ctx.data.0 * 2)
}

fn scaled_system(ctx: SignalCtx<&Source>, scale: Res<Scale>) -> Doubled {
    Doubled(ctx.data.0 * scale.0)
}

/// Writes through the ctx's queue on the way to returning a value, which is the
/// whole reason a mapper is a system rather than a function.
fn echoing_system(mut ctx: SignalCtx<&Source>, sink: Res<Sink>) -> Doubled {
    let value = ctx.data.0;
    ctx.commands.entity(sink.0).insert(Echo(value));
    Doubled(value * 2)
}

/// Every registered mapper entity for `D`, whatever it maps to.
fn registered<D: SignalData>(world: &mut World) -> Vec<Entity> {
    world
        .query_filtered::<Entity, With<RegisteredSignalSystem<D>>>()
        .iter(world)
        .collect()
}

/// How many mapper-system subscriptions `entity` carries for `D`.
fn system_subscribers<D: SignalData>(world: &World, entity: Entity) -> usize
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    world
        .get::<SubscriberSet<D>>(entity)
        .map_or(0, |set| set.system_len())
}

/// How many subscribers are currently claiming `system`.
fn users<D: SignalData>(world: &World, system: Entity) -> Option<usize> {
    world
        .get::<RegisteredSignalSystem<D>>(system)
        .map(|registered| registered.users)
}

/// Same debt a plain map settles on attach: the scan only visits what changed.
#[test]
fn a_mapper_system_evaluates_on_insertion() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(21)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map_system(double_system)).id()
    };
    world.flush();

    // No schedule run: the value must already be there.
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(42)));
}

#[test]
fn a_mapper_system_maps_again_when_the_source_changes() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map_system(double_system)).id()
    };
    world.flush();
    run_reactive_schedule(world);

    world.get_mut::<Source>(source).unwrap().0 = 5;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(10)));
}

/// The mapper's own `SystemParam`s resolve against the live world, on the
/// attach-time run and on every scan run after it.
#[test]
fn a_mapper_system_reads_its_own_params() {
    let mut app = app();
    let world = app.world_mut();
    world.insert_resource(Scale(10));
    let source = world.spawn(Source(3)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map_system(scaled_system)).id()
    };
    world.flush();
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(30)));
    run_reactive_schedule(world);

    world.insert_resource(Scale(100));
    world.get_mut::<Source>(source).unwrap().0 = 4;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(400)));
}

/// The point of the whole exercise: a mapper runs under `&World` inside the
/// scan, so its commands have to reach the scan's own queue to ever apply.
#[test]
fn a_mapper_system_queues_commands() {
    let mut app = app();
    let world = app.world_mut();
    let sink = world.spawn_empty().id();
    world.insert_resource(Sink(sink));
    let source = world.spawn(Source(7)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map_system(echoing_system)).id()
    };
    world.flush();

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(14)));
    assert_eq!(
        world.get::<Echo>(sink),
        Some(&Echo(7)),
        "the attach-time run's commands must apply"
    );
    run_reactive_schedule(world);

    world.get_mut::<Source>(source).unwrap().0 = 9;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(18)));
    assert_eq!(
        world.get::<Echo>(sink),
        Some(&Echo(9)),
        "the scan's commands must apply too"
    );
}

/// A mapper's output is an ordinary component, so a second signal can watch it.
/// This only settles in one frame if a mapper run counts as a dispatch — the
/// pass that writes `Doubled` has to be followed by one that sees it.
#[test]
fn a_mapper_system_chain_settles_in_one_frame() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(3)).id();

    let (first, second) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let first = commands.spawn(signal.map_system(double_system)).id();

        let doubled = commands.signal::<&Doubled>().watch(first);
        let second = commands.spawn(doubled.map(quadruple)).id();
        (first, second)
    };
    world.flush();
    run_reactive_schedule(world);

    world.get_mut::<Source>(source).unwrap().0 = 10;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(first), Some(&Doubled(20)));
    assert_eq!(
        world.get::<Quadrupled>(second),
        Some(&Quadrupled(40)),
        "the second hop must land in the same frame as the first"
    );
}

/// A mapper run has to count towards the pass's dispatch tally, exactly like a
/// closure does.
///
/// The tally is the settle loop's only stopping condition: a pass that reports
/// zero is taken as proof the graph is quiet, and the loop breaks before running
/// the pass that would have observed what the mapper just wrote. Nothing catches
/// that in a chain whose hops happen to be registered in dependency order — the
/// downstream scan runs later in the *same* pass — so it is checked here on the
/// pass count itself.
#[test]
fn a_mapper_run_counts_as_a_dispatch() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(3)).id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map_system(double_system));
    }
    world.flush();
    run_reactive_schedule(world);

    // Nothing changed, so the very first pass settles.
    assert_eq!(settle_reactive(world), 1, "a quiet frame costs one pass");

    world.get_mut::<Source>(source).unwrap().0 = 10;
    assert_eq!(
        settle_reactive(world),
        2,
        "the mapper dispatched, so a second pass has to prove the graph settled"
    );
}

/// A `fn` item is zero-sized, so two subscribers using it are asking for the
/// same thing and can share one instance — and the `QueryState` behind it.
#[test]
fn zero_sized_mappers_share_one_registration() {
    let mut app = app();
    let world = app.world_mut();
    let left = world.spawn(Source(1)).id();
    let right = world.spawn(Source(2)).id();

    let (first, second) = {
        let mut commands = world.commands();
        let left_signal = commands.signal::<&Source>().watch(left);
        let first = commands.spawn(left_signal.map_system(double_system)).id();
        let right_signal = commands.signal::<&Source>().watch(right);
        let second = commands.spawn(right_signal.map_system(double_system)).id();
        (first, second)
    };
    world.flush();

    let systems = registered::<&Source>(world);
    assert_eq!(systems.len(), 1, "one instance serves both subscribers");
    assert_eq!(users::<&Source>(world, systems[0]), Some(2));

    // Sharing must not blur the two targets together.
    assert_eq!(world.get::<Doubled>(first), Some(&Doubled(2)));
    assert_eq!(world.get::<Doubled>(second), Some(&Doubled(4)));
    run_reactive_schedule(world);

    world.get_mut::<Source>(right).unwrap().0 = 8;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(first), Some(&Doubled(2)));
    assert_eq!(world.get::<Doubled>(second), Some(&Doubled(16)));
}

/// The capture is what makes two closures of the same type behave differently,
/// so they cannot share.
#[test]
fn capturing_mappers_get_their_own_registration() {
    let mut app = app();
    let world = app.world_mut();
    let sources = [world.spawn(Source(10)).id(), world.spawn(Source(20)).id()];

    let mut targets = Vec::new();
    {
        let mut commands = world.commands();
        for (index, source) in sources.into_iter().enumerate() {
            let offset = index as i32 + 1;
            let signal = commands.signal::<&Source>().watch(source);
            // One closure type, two different captures.
            targets.push(
                commands
                    .spawn(
                        signal.map_system(move |ctx: SignalCtx<&Source>| {
                            Doubled(ctx.data.0 + offset)
                        }),
                    )
                    .id(),
            );
        }
    }
    world.flush();

    assert_eq!(registered::<&Source>(world).len(), 2);
    assert_eq!(world.get::<Doubled>(targets[0]), Some(&Doubled(11)));
    assert_eq!(world.get::<Doubled>(targets[1]), Some(&Doubled(22)));
}

/// A registration outlives its subscribers only until the last one lets go.
#[test]
fn a_dropped_subscriber_releases_its_registration() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let (first, second) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let first = commands.spawn(signal.map_system(double_system)).id();
        let signal = commands.signal::<&Source>().watch(source);
        let second = commands.spawn(signal.map_system(double_system)).id();
        (first, second)
    };
    world.flush();

    let systems = registered::<&Source>(world);
    assert_eq!(systems.len(), 1);
    assert_eq!(users::<&Source>(world, systems[0]), Some(2));
    assert_eq!(
        subscribers::<&Source>(world, source),
        0,
        "no closures, only systems"
    );

    world.despawn(first);
    world.flush();
    assert_eq!(
        registered::<&Source>(world).len(),
        1,
        "the surviving subscriber still needs it"
    );
    assert_eq!(users::<&Source>(world, systems[0]), Some(1));

    world.despawn(second);
    world.flush();
    assert!(
        registered::<&Source>(world).is_empty(),
        "the last release despawns the registration"
    );

    // And the source must not still be trying to run it.
    world.get_mut::<Source>(source).unwrap().0 = 5;
    run_reactive_schedule(world);
}

/// A dropped subscriber stops being dispatched, and does not take its
/// still-subscribed neighbours down with it.
#[test]
fn a_dropped_subscriber_stops_being_dispatched() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let (dropped, kept) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let dropped = commands.spawn(signal.map_system(double_system)).id();
        let signal = commands.signal::<&Source>().watch(source);
        let kept = commands.spawn(signal.map_system(double_system)).id();
        (dropped, kept)
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(system_subscribers::<&Source>(world, source), 2);

    world.despawn(dropped);
    world.flush();
    assert_eq!(
        system_subscribers::<&Source>(world, source),
        1,
        "the source must stop dispatching to a subscriber that is gone"
    );

    world.get_mut::<Source>(source).unwrap().0 = 50;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(kept), Some(&Doubled(100)));
}
