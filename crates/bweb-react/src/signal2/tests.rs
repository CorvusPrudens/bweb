use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use bevy_app::App;
use bevy_ecs::{prelude::*, system::RunSystemOnce, system::SystemIdMarker};

use crate::signal2::{
    REACTION_LIMIT, ReactError, ReactivePlugin, RegisteredSignalSystem, SignalCtx, SignalData,
    SignalExt, SubscriberSet,
    derived::{NodeLevel, NodeSources, NodeSubscribers},
    dynamic::Signal,
    effect::Effect,
    list::{ListOf, ListSource, ReactiveList},
    run_reactive_schedule, settle_reactive,
    source::{SignalStore, SourceSignal, WatchTarget, Watching},
    value::Derived,
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

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Selected(Entity);

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Chosen(Entity);

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

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
struct EffectLog {
    runs: usize,
    values: Vec<i32>,
}

impl EffectLog {
    fn record(&mut self, value: i32) {
        self.runs += 1;
        self.values.push(value);
    }
}

fn effect_log(world: &World) -> &EffectLog {
    world.resource::<EffectLog>()
}

/// An effect is seeded into the next pass the moment it is inserted, so it gets
/// a first run without anything having changed — the same debt a derived node
/// settles, and for the same reason.
#[test]
fn an_effect_runs_on_the_first_pass() {
    let mut app = app();
    let world = app.world_mut();
    world.init_resource::<EffectLog>();
    let source = world.spawn(Source(3)).id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(Effect::new(
            move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                if let Ok(source) = signal.get(&store) {
                    log.record(source.0);
                }
            },
        ));
    }
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(effect_log(world).values, vec![3]);
    assert_eq!(
        effect_log(world).runs,
        1,
        "attaching must run the effect exactly once"
    );
}

#[test]
fn an_effect_re_runs_when_a_signal_it_read_changes() {
    let mut app = app();
    let world = app.world_mut();
    world.init_resource::<EffectLog>();
    let source = world.spawn(Source(1)).id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(Effect::new(
            move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                if let Ok(source) = signal.get(&store) {
                    log.record(source.0);
                }
            },
        ));
    }
    world.flush();
    run_reactive_schedule(world);

    world.get_mut::<Source>(source).unwrap().0 = 2;
    run_reactive_schedule(world);
    world.get_mut::<Source>(source).unwrap().0 = 3;
    run_reactive_schedule(world);

    assert_eq!(effect_log(world).values, vec![1, 2, 3]);
}

/// A quiet frame must not wake it. Auto-tracking is only worth anything if the
/// edges it wires are the ones it actually reads.
#[test]
fn an_effect_stays_quiet_when_nothing_it_reads_changes() {
    let mut app = app();
    let world = app.world_mut();
    world.init_resource::<EffectLog>();
    let source = world.spawn(Source(1)).id();
    let unrelated = world.spawn(Other(1)).id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(Effect::new(
            move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                if let Ok(source) = signal.get(&store) {
                    log.record(source.0);
                }
            },
        ));
    }
    world.flush();
    run_reactive_schedule(world);
    let settled = effect_log(world).runs;

    world.get_mut::<Other>(unrelated).unwrap().0 = 99;
    run_reactive_schedule(world);
    run_reactive_schedule(world);

    assert_eq!(effect_log(world).runs, settled);
}

/// One body, many placements: the entity is what tells a shared system which of
/// them this run is for.
#[test]
fn an_effect_receives_the_entity_it_is_placed_on() {
    /// The same body, placed twice. Whatever it writes has to land on the
    /// entity it was placed on rather than on some entity captured up front.
    fn body(
        signal: SourceSignal<&'static Source>,
    ) -> impl Fn(In<Entity>, SignalStore, Commands) + Send + Sync + 'static {
        move |In(entity), store, mut commands| {
            if let Ok(source) = signal.get(&store) {
                commands.entity(entity).insert(Doubled(source.0 * 2));
            }
        }
    }

    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(21)).id();

    let (first, second) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let first = commands.spawn(Effect::new(body(signal.clone()))).id();
        let second = commands.spawn(Effect::new(body(signal))).id();
        (first, second)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(first), Some(&Doubled(42)));
    assert_eq!(world.get::<Doubled>(second), Some(&Doubled(42)));
}

/// The whole point of running effects exclusively rather than dispatching them
/// off the scan: an effect is an ordinary system, so it can take `ResMut` and
/// mutate the world directly. A read-only mapper running under `&World` cannot.
#[test]
fn an_effect_can_mutate_resources_directly() {
    let mut app = app();
    let world = app.world_mut();
    world.insert_resource(Scale(0));
    let source = world.spawn(Source(3)).id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(Effect::new(
            move |_: In<Entity>, store: SignalStore, mut scale: ResMut<Scale>| {
                if let Ok(source) = signal.get(&store) {
                    scale.0 = source.0 * 10;
                }
            },
        ));
    }
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.resource::<Scale>().0, 30);
}

/// Two inputs changing in one frame is one evaluation, not one per input —
/// effects inherit the derived pass's dedup by going through it.
#[test]
fn an_effect_runs_once_for_two_changed_inputs() {
    let mut app = app();
    let world = app.world_mut();
    world.init_resource::<EffectLog>();
    let left = world.spawn(Source(1)).id();
    let right = world.spawn(Other(10)).id();

    {
        let mut commands = world.commands();
        let a = commands.signal::<&Source>().watch(left);
        let b = commands.signal::<&Other>().watch(right);
        commands.spawn(Effect::new(
            move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                if let (Ok(a), Ok(b)) = (a.get(&store), b.get(&store)) {
                    log.record(a.0 + b.0);
                }
            },
        ));
    }
    world.flush();
    run_reactive_schedule(world);
    let settled = effect_log(world).runs;

    world.get_mut::<Source>(left).unwrap().0 = 2;
    world.get_mut::<Other>(right).unwrap().0 = 20;
    run_reactive_schedule(world);

    assert_eq!(effect_log(world).runs - settled, 1);
    assert_eq!(effect_log(world).values.last(), Some(&22));
}

/// Lazy tracking has to cut both ways for an effect too, or a conditional
/// effect degrades into one that fires on the union of everything it has ever
/// touched.
#[test]
fn an_effect_drops_the_edges_it_stops_reading() {
    let mut app = app();
    let world = app.world_mut();
    world.init_resource::<EffectLog>();
    let left_entity = world.spawn(Left(1)).id();
    let right_entity = world.spawn(Right(100)).id();

    let read_left = Arc::new(AtomicBool::new(true));
    let branch = read_left.clone();

    {
        let mut commands = world.commands();
        let left = commands.signal::<&Left>().watch(left_entity);
        let right = commands.signal::<&Right>().watch(right_entity);
        commands.spawn(Effect::new(
            move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                let value = if branch.load(Ordering::Relaxed) {
                    left.get(&store).map(|left| left.0)
                } else {
                    right.get(&store).map(|right| right.0)
                };

                if let Ok(value) = value {
                    log.record(value);
                }
            },
        ));
    }
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(effect_log(world).values.last(), Some(&1));
    assert_eq!(subscribers::<&Left>(world, left_entity), 1);
    assert_eq!(subscribers::<&Right>(world, right_entity), 0);

    // Flip the branch, and poke the input it is still reading to wake it.
    read_left.store(false, Ordering::Relaxed);
    world.get_mut::<Left>(left_entity).unwrap().0 = 2;
    run_reactive_schedule(world);

    assert_eq!(effect_log(world).values.last(), Some(&100));
    assert_eq!(subscribers::<&Left>(world, left_entity), 0);
    assert_eq!(subscribers::<&Right>(world, right_entity), 1);

    let settled = effect_log(world).runs;
    world.get_mut::<Left>(left_entity).unwrap().0 = 3;
    run_reactive_schedule(world);
    assert_eq!(
        effect_log(world).runs,
        settled,
        "the abandoned input must no longer wake the effect"
    );

    world.get_mut::<Right>(right_entity).unwrap().0 = 200;
    run_reactive_schedule(world);
    assert_eq!(effect_log(world).values.last(), Some(&200));
}

/// Removing the component ends the effect: the subscriptions come off and the
/// system it registered goes with them. The system lives on an entity of its
/// own, so nothing else would ever collect it.
#[test]
fn a_removed_effect_unsubscribes_and_unregisters() {
    let mut app = app();
    let world = app.world_mut();
    world.init_resource::<EffectLog>();
    let source = world.spawn(Source(1)).id();

    let host = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands
            .spawn(Effect::new(
                move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                    if let Ok(source) = signal.get(&store) {
                        log.record(source.0);
                    }
                },
            ))
            .id()
    };
    world.flush();
    run_reactive_schedule(world);

    let registered = world
        .get::<Effect>(host)
        .and_then(|effect| effect.registered)
        .expect("the effect should have registered its system");
    assert!(world.get_entity(registered.entity()).is_ok());
    assert_eq!(subscribers::<&Source>(world, source), 1);

    world.entity_mut(host).remove::<Effect>();
    world.flush();

    assert_eq!(subscribers::<&Source>(world, source), 0);
    assert!(
        world.get_entity(registered.entity()).is_err(),
        "the registered system must not outlive the effect"
    );

    let settled = effect_log(world).runs;
    world.get_mut::<Source>(source).unwrap().0 = 9;
    run_reactive_schedule(world);
    assert_eq!(effect_log(world).runs, settled);
}

#[test]
fn a_despawned_effect_unsubscribes_and_unregisters() {
    let mut app = app();
    let world = app.world_mut();
    world.init_resource::<EffectLog>();
    let source = world.spawn(Source(1)).id();

    let host = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands
            .spawn(Effect::new(
                move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                    if let Ok(source) = signal.get(&store) {
                        log.record(source.0);
                    }
                },
            ))
            .id()
    };
    world.flush();
    run_reactive_schedule(world);

    let registered = world
        .get::<Effect>(host)
        .and_then(|effect| effect.registered)
        .expect("the effect should have registered its system");

    world.despawn(host);
    world.flush();

    assert_eq!(subscribers::<&Source>(world, source), 0);
    assert!(world.get_entity(registered.entity()).is_err());

    // And the source must still be usable afterwards.
    let settled = effect_log(world).runs;
    world.get_mut::<Source>(source).unwrap().0 = 5;
    run_reactive_schedule(world);
    assert_eq!(effect_log(world).runs, settled);
}

/// Registration is deferred, so an effect can be gone before it ever happens.
/// The system it would have registered lives on an entity of its own, which
/// nothing else will ever collect — so the two bail-outs on that path have to
/// clean up after themselves.
#[test]
fn an_effect_that_ends_before_it_registers_leaks_nothing() {
    fn registered_systems(world: &mut World) -> usize {
        world
            .query_filtered::<Entity, With<SystemIdMarker>>()
            .iter(world)
            .count()
    }

    for despawn_host in [false, true] {
        let mut app = app();
        let world = app.world_mut();
        world.init_resource::<EffectLog>();
        let source = world.spawn(Source(1)).id();
        let before = registered_systems(world);

        {
            let mut commands = world.commands();
            let signal = commands.signal::<&Source>().watch(source);
            let host = commands
                .spawn(Effect::new(
                    move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                        if let Ok(source) = signal.get(&store) {
                            log.record(source.0);
                        }
                    },
                ))
                .id();

            // Queued behind the insert, so it lands before the deferred
            // registration the insert asked for.
            if despawn_host {
                commands.entity(host).despawn();
            } else {
                commands.entity(host).remove::<Effect>();
            }
        }
        world.flush();
        run_reactive_schedule(world);

        assert_eq!(
            registered_systems(world),
            before,
            "despawn_host = {despawn_host}"
        );
        assert_eq!(effect_log(world).runs, 0);
        assert_eq!(subscribers::<&Source>(world, source), 0);
    }
}

/// Overwriting an effect has to end the old one first, or the entity ends up
/// subscribed twice and running two bodies on every change.
#[test]
fn re_inserting_an_effect_replaces_it() {
    let mut app = app();
    let world = app.world_mut();
    world.init_resource::<EffectLog>();
    let source = world.spawn(Source(1)).id();

    let (signal, host) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let watched = signal.clone();
        let host = commands
            .spawn(Effect::new(
                move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                    if let Ok(source) = watched.get(&store) {
                        log.record(source.0);
                    }
                },
            ))
            .id();
        (signal, host)
    };
    world.flush();
    run_reactive_schedule(world);

    let first = world
        .get::<Effect>(host)
        .and_then(|effect| effect.registered)
        .expect("the effect should have registered its system");
    assert_eq!(subscribers::<&Source>(world, source), 1);

    world.entity_mut(host).insert(Effect::new(
        move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
            if let Ok(source) = signal.get(&store) {
                log.record(source.0 * 1000);
            }
        },
    ));
    world.flush();
    run_reactive_schedule(world);

    assert!(
        world.get_entity(first.entity()).is_err(),
        "the replaced body's system must be unregistered"
    );
    assert_eq!(
        subscribers::<&Source>(world, source),
        1,
        "the old subscription must go before the new one lands"
    );

    let settled = effect_log(world).runs;
    world.get_mut::<Source>(source).unwrap().0 = 7;
    run_reactive_schedule(world);

    assert_eq!(effect_log(world).runs - settled, 1, "one body, not two");
    assert_eq!(effect_log(world).values.last(), Some(&7000));
}

/// An effect writing an ordinary component is just another signal source, so a
/// mapped signal downstream of one has to land in the same frame.
#[test]
fn a_signal_downstream_of_an_effect_settles_in_one_frame() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(3)).id();

    let (host, mapped) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let host = commands
            .spawn(Effect::new(
                move |In(entity): In<Entity>, store: SignalStore, mut commands: Commands| {
                    if let Ok(source) = signal.get(&store) {
                        commands.entity(entity).insert(Doubled(source.0 * 2));
                    }
                },
            ))
            .id();

        let doubled = commands.signal::<&Doubled>().watch(host);
        let mapped = commands.spawn(doubled.map(quadruple)).id();
        (host, mapped)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(host), Some(&Doubled(6)));
    assert_eq!(world.get::<Quadrupled>(mapped), Some(&Quadrupled(12)));

    world.get_mut::<Source>(source).unwrap().0 = 10;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(host), Some(&Doubled(20)));
    assert_eq!(
        world.get::<Quadrupled>(mapped),
        Some(&Quadrupled(40)),
        "the hop downstream of the effect must land in the same frame"
    );
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

// -- lists -------------------------------------------------------------------

/// A plain `Vec` newtype standing in for whatever collection component a caller
/// actually has. Not a relationship target, so it exercises the manual impl
/// rather than the blanket.
#[derive(Component, Clone)]
struct Items(Vec<u32>);

impl ListSource for Items {
    type Item = u32;

    fn items(&self) -> &[u32] {
        &self.0
    }
}

/// Keys and values pulled apart, so a retained key can carry a changed value.
#[derive(Component, Clone)]
struct Pairs(Vec<(u32, u32)>);

impl ListSource for Pairs {
    type Item = (u32, u32);

    fn items(&self) -> &[(u32, u32)] {
        &self.0
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct RowValue(u32);

/// The container's rows in relationship order, as the values their bodies wrote.
fn row_values(world: &mut World, container: Entity) -> Vec<u32> {
    world
        .get::<Children>(container)
        .map(|children| children.iter().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entity| world.get::<RowValue>(entity).map(|value| value.0))
        .collect()
}

fn children(world: &mut World, container: Entity) -> Vec<Entity> {
    world
        .get::<Children>(container)
        .map(|children| children.iter().collect())
        .unwrap_or_default()
}

/// A model entity holding `initial`, and an empty container carrying a list
/// over it, already settled.
fn list_app(initial: Vec<u32>) -> (App, Entity, Entity) {
    let mut app = app();
    let world = app.world_mut();
    let model = world.spawn(Items(initial)).id();
    let container = world.spawn_empty().id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Items>().watch(model);
        let list: ReactiveList = commands.list(signal, |item| *item, |item, _| RowValue(*item));
        commands.entity(container).insert(list);
    }
    world.flush();
    run_reactive_schedule(world);

    (app, model, container)
}

#[test]
fn a_list_builds_its_rows_in_order() {
    let (mut app, _, container) = list_app(vec![1, 2, 3]);
    let world = app.world_mut();

    assert_eq!(row_values(world, container), vec![1, 2, 3]);
}

/// The point of keying: a reorder moves the entities that already exist rather
/// than despawning and respawning them.
#[test]
fn a_reorder_moves_rows_without_respawning_them() {
    let (mut app, model, container) = list_app(vec![1, 2, 3]);
    let world = app.world_mut();

    let mut before = children(world, container);

    world.get_mut::<Items>(model).unwrap().0 = vec![3, 1, 2];
    run_reactive_schedule(world);

    assert_eq!(row_values(world, container), vec![3, 1, 2]);

    let mut after = children(world, container);
    before.sort();
    after.sort();
    assert_eq!(before, after, "a reorder must not respawn rows");
}

#[test]
fn an_insert_lands_at_its_collection_position() {
    let (mut app, model, container) = list_app(vec![1, 4]);
    let world = app.world_mut();
    assert_eq!(row_values(world, container), vec![1, 4]);

    world.get_mut::<Items>(model).unwrap().0 = vec![1, 2, 3, 4];
    run_reactive_schedule(world);
    assert_eq!(row_values(world, container), vec![1, 2, 3, 4]);

    // And a removal keeps the survivors where they were.
    world.get_mut::<Items>(model).unwrap().0 = vec![2, 4];
    run_reactive_schedule(world);
    assert_eq!(row_values(world, container), vec![2, 4]);
}

/// The list owns its own rows and nothing else: entities the caller put on the
/// container keep their places across a reorder.
#[test]
fn a_list_preserves_static_siblings() {
    let mut app = app();
    let world = app.world_mut();
    let model = world.spawn(Items(vec![1, 2])).id();

    let container = world.spawn_empty().id();
    let static_child = world.spawn((ChildOf(container), RowValue(99))).id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Items>().watch(model);
        let list: ReactiveList = commands.list(signal, |item| *item, |item, _| RowValue(*item));
        commands.entity(container).insert(list);
    }
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(row_values(world, container), vec![99, 1, 2]);

    world.get_mut::<Items>(model).unwrap().0 = vec![2, 1];
    run_reactive_schedule(world);

    assert_eq!(row_values(world, container), vec![99, 2, 1]);
    assert_eq!(
        children(world, container).first().copied(),
        Some(static_child),
        "the static sibling must keep its position"
    );
}

/// The case v1 never made work: a retained key whose *value* moved rebuilds the
/// row it already has, in place.
#[test]
fn a_retained_row_is_rebuilt_in_place() {
    let mut app = app();
    let world = app.world_mut();
    let model = world.spawn(Pairs(vec![(1, 10), (2, 20)])).id();
    let container = world.spawn_empty().id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Pairs>().watch(model);
        let list: ReactiveList =
            commands.list(signal, |(key, _)| *key, |(_, value), _| RowValue(*value));
        commands.entity(container).insert(list);
    }
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(row_values(world, container), vec![10, 20]);
    let before = children(world, container);

    world.get_mut::<Pairs>(model).unwrap().0 = vec![(1, 11), (2, 20)];
    run_reactive_schedule(world);

    assert_eq!(row_values(world, container), vec![11, 20]);
    assert_eq!(
        children(world, container),
        before,
        "an update must not respawn or reorder rows"
    );
}

/// A collection component that changed without its contents moving must not
/// touch a single row. The row body is the expensive part of a list, and this
/// is the whole reason the diff keeps a per-row value baseline.
#[test]
fn an_unchanged_collection_runs_no_row_bodies() {
    let mut app = app();
    let world = app.world_mut();
    let model = world.spawn(Items(vec![1, 2, 3])).id();
    let container = world.spawn_empty().id();

    let builds = Arc::new(AtomicUsize::new(0));

    {
        let counter = builds.clone();
        let mut commands = world.commands();
        let signal = commands.signal::<&Items>().watch(model);
        let list: ReactiveList = commands.list(
            signal,
            |item| *item,
            move |item, _| {
                counter.fetch_add(1, Ordering::Relaxed);
                RowValue(*item)
            },
        );
        commands.entity(container).insert(list);
    }
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(builds.load(Ordering::Relaxed), 3);

    // Touched, so the effect re-runs — but the contents are identical.
    world.get_mut::<Items>(model).unwrap().0 = vec![1, 2, 3];
    run_reactive_schedule(world);

    assert_eq!(
        builds.load(Ordering::Relaxed),
        3,
        "an identical collection must not rebuild any row"
    );
    assert_eq!(row_values(world, container), vec![1, 2, 3]);
}

/// Ending the list ends everything it owns: the rows, the effect's
/// subscriptions, and the system the effect registered.
#[test]
fn a_removed_list_despawns_its_rows_and_unregisters() {
    let (mut app, model, container) = list_app(vec![1, 2, 3]);
    let world = app.world_mut();

    let rows = children(world, container);
    assert_eq!(rows.len(), 3);
    assert_eq!(subscribers::<&Items>(world, model), 1);

    let controller = world
        .query_filtered::<Entity, With<ListOf>>()
        .single(world)
        .expect("the list should have spawned exactly one controller");
    let registered = world
        .get::<Effect>(controller)
        .and_then(|effect| effect.registered)
        .expect("the controller's effect should have registered its system");

    world.entity_mut(container).remove::<ReactiveList>();
    world.flush();

    for row in rows {
        assert!(
            world.get_entity(row).is_err(),
            "a row must not outlive the list"
        );
    }
    assert!(world.get_entity(controller).is_err());
    assert!(
        world.get_entity(registered.entity()).is_err(),
        "the registered system must not outlive the list"
    );
    assert_eq!(subscribers::<&Items>(world, model), 0);

    // And the source must still be usable afterwards.
    world.get_mut::<Items>(model).unwrap().0 = vec![9];
    run_reactive_schedule(world);
    assert_eq!(children(world, container), Vec::new());
}

/// Two lists in one world own strictly their own rows: a change to one leaves
/// the other's container untouched, including its collection order.
#[test]
fn two_lists_do_not_disturb_each_other() {
    #[derive(Component, Clone)]
    struct Others(Vec<u32>);

    impl ListSource for Others {
        type Item = u32;

        fn items(&self) -> &[u32] {
            &self.0
        }
    }

    let mut app = app();
    let world = app.world_mut();
    let left_model = world.spawn(Items(vec![1, 2])).id();
    let right_model = world.spawn(Others(vec![7, 8])).id();
    let left = world.spawn_empty().id();
    let right = world.spawn_empty().id();

    {
        let mut commands = world.commands();
        let a = commands.signal::<&Items>().watch(left_model);
        let first: ReactiveList = commands.list(a, |item| *item, |item, _| RowValue(*item));
        commands.entity(left).insert(first);

        let b = commands.signal::<&Others>().watch(right_model);
        let second: ReactiveList = commands.list(b, |item| *item, |item, _| RowValue(*item));
        commands.entity(right).insert(second);
    }
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(row_values(world, left), vec![1, 2]);
    assert_eq!(row_values(world, right), vec![7, 8]);

    let right_rows = children(world, right);

    world.get_mut::<Items>(left_model).unwrap().0 = vec![2, 1];
    run_reactive_schedule(world);

    assert_eq!(row_values(world, left), vec![2, 1]);
    assert_eq!(row_values(world, right), vec![7, 8]);
    assert_eq!(
        children(world, right),
        right_rows,
        "the quiet list's rows must not have been touched at all"
    );
}

/// One container holds one list *per relationship type* — `ReactiveList<R>` is a
/// single component, so a second one for the same `R` overwrites the first
/// rather than joining it. What matters is that the overwrite is a clean
/// handover: the first list's rows and effect go, and the second's replace them.
#[test]
fn a_second_list_replaces_the_first() {
    let (mut app, _, container) = list_app(vec![1, 2, 3]);
    let world = app.world_mut();

    let first_rows = children(world, container);
    assert_eq!(row_values(world, container), vec![1, 2, 3]);

    let other_model = world.spawn(Items(vec![7, 8])).id();
    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Items>().watch(other_model);
        let second: ReactiveList = commands.list(signal, |item| *item, |item, _| RowValue(*item));
        commands.entity(container).insert(second);
    }
    world.flush();
    run_reactive_schedule(world);

    for row in first_rows {
        assert!(
            world.get_entity(row).is_err(),
            "the replaced list's rows must not survive as orphans"
        );
    }
    assert_eq!(row_values(world, container), vec![7, 8]);

    // Exactly one controller is left: the replaced list's was despawned.
    assert_eq!(
        world
            .query_filtered::<Entity, With<ListOf>>()
            .iter(world)
            .count(),
        1
    );
}

/// A list whose source is a relationship collection: the blanket impl, and the
/// one shape that can feed its own output back into its input.
#[test]
fn a_list_sourced_from_children_settles() {
    let mut app = app();
    let world = app.world_mut();

    let model = world.spawn_empty().id();
    for value in [1, 2, 3] {
        world.spawn((ChildOf(model), RowValue(value)));
    }
    let container = world.spawn_empty().id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Children>().watch(model);
        // One row per source child, pointing back at the child it stands for.
        let list: ReactiveList = commands.list(signal, |child| *child, |child, _| Mirrors(*child));
        commands.entity(container).insert(list);
    }
    world.flush();
    run_reactive_schedule(world);

    let mirrored: Vec<Entity> = children(world, container)
        .into_iter()
        .filter_map(|row| world.get::<Mirrors>(row).map(|m| m.0))
        .collect();
    assert_eq!(mirrored, children(world, model));

    // A quiet frame must cost one pass: nothing here may keep waking itself.
    assert_eq!(
        settle_reactive(world),
        1,
        "a settled list must not re-dirty its own source"
    );
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Mirrors(Entity);

/// A list whose container is itself a row of another list. The inner list is
/// built by a row body, so it only exists from the pass after the outer one ran.
#[test]
fn nested_lists_settle() {
    let mut app = app();
    let world = app.world_mut();
    let outer_model = world.spawn(Items(vec![1, 2])).id();
    let inner_model = world.spawn(Items(vec![10, 20, 30])).id();
    let container = world.spawn_empty().id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Items>().watch(outer_model);
        let list: ReactiveList = commands.list(
            signal,
            |item| *item,
            move |item, commands| {
                let inner_signal = commands.signal::<&Items>().watch(inner_model);
                let inner: ReactiveList =
                    commands.list(inner_signal, |item| *item, |item, _| RowValue(*item));
                (RowValue(*item), inner)
            },
        );
        commands.entity(container).insert(list);
    }
    world.flush();
    let passes = settle_reactive(world);

    let rows = children(world, container);
    assert_eq!(rows.len(), 2);
    assert_eq!(row_values(world, container), vec![1, 2]);

    for row in rows {
        assert_eq!(
            row_values(world, row),
            vec![10, 20, 30],
            "each nested list must have built its own rows"
        );
    }

    // Three, and the breakdown is worth stating because it is the cost of one
    // level of nesting: the outer effect runs and queues the rows, the inner
    // effects attach and run, and a third pass proves there is nothing left. A
    // deeper tree costs one more pass per level, against a budget of
    // `REACTION_LIMIT`.
    assert_eq!(passes, 3, "one level of nesting costs one extra pass");
    assert!(passes < REACTION_LIMIT);
    assert_eq!(settle_reactive(world), 1, "and must then stay quiet");

    world.get_mut::<Items>(inner_model).unwrap().0 = vec![10, 30];
    run_reactive_schedule(world);
    for row in children(world, container) {
        assert_eq!(row_values(world, row), vec![10, 30]);
    }
}

/// How many derived nodes are subscribed to `cell`.
fn cell_subscribers(world: &World, cell: Entity) -> usize {
    world
        .get::<NodeSubscribers>(cell)
        .map_or(0, |subscribers| subscribers.0.len())
}

/// The whole point of a cell: the value is in the handle, so a writer can read
/// back what it just wrote without waiting for a flush. A component-backed
/// signal writes through `Commands` and would still be showing the old value
/// here.
#[test]
fn a_cell_write_is_visible_immediately() {
    let mut app = app();
    let world = app.world_mut();

    let cell = world.commands().cell(1);
    assert_eq!(*cell.peek(), 1);

    cell.set(2);
    assert_eq!(*cell.peek(), 2, "no flush, no schedule run — still visible");

    cell.update(|value| *value += 40);
    assert_eq!(*cell.peek(), 42);

    // And the entity behind it does not even have to exist yet.
    world.flush();
    assert_eq!(*cell.peek(), 42);
}

#[test]
fn a_derived_node_reads_a_cell_without_a_change() {
    let mut app = app();
    let world = app.world_mut();

    let node = {
        let mut commands = world.commands();
        let cell = commands.cell(21);
        commands.derive(move |s| Ok(Sum(*cell.read(s) * 2)))
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(42)));
}

#[test]
fn writing_a_cell_wakes_its_readers() {
    let mut app = app();
    let world = app.world_mut();

    let (cell, node) = {
        let mut commands = world.commands();
        let cell = commands.cell(1);
        let node = commands.derive({
            let cell = cell.clone();
            move |s| Ok(Sum(*cell.read(s)))
        });
        (cell, node)
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(1)));
    assert_eq!(cell_subscribers(world, cell.entity()), 1);

    cell.set(9);
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(9)));
}

/// A pass that marks nothing must still be cheap, and — more importantly — a
/// cell nobody wrote must not wake anything.
#[test]
fn a_quiet_cell_wakes_nobody() {
    let mut app = app();
    let world = app.world_mut();

    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();

    let cell = {
        let mut commands = world.commands();
        let cell = commands.cell(1);
        commands.derive({
            let cell = cell.clone();
            move |s| {
                counter.fetch_add(1, Ordering::Relaxed);
                Ok(Sum(*cell.read(s)))
            }
        });
        cell
    };
    world.flush();
    run_reactive_schedule(world);

    let settled = runs.load(Ordering::Relaxed);
    for _ in 0..3 {
        run_reactive_schedule(world);
    }
    assert_eq!(runs.load(Ordering::Relaxed), settled);

    // A `write` guard that is never dereferenced mutably changes nothing, so it
    // must not count as a write either.
    drop(cell.write());
    run_reactive_schedule(world);
    assert_eq!(runs.load(Ordering::Relaxed), settled);
}

#[test]
fn peeking_does_not_subscribe() {
    let mut app = app();
    let world = app.world_mut();

    let (cell, node) = {
        let mut commands = world.commands();
        let cell = commands.cell(1);
        let node = commands.derive({
            let cell = cell.clone();
            move |_| Ok(Sum(*cell.peek()))
        });
        (cell, node)
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(1)));
    assert_eq!(cell_subscribers(world, cell.entity()), 0);

    cell.set(9);
    run_reactive_schedule(world);
    assert_eq!(
        world.get::<Sum>(node.entity()),
        Some(&Sum(1)),
        "an unrecorded read must not wake the node"
    );
}

/// Two cells changing in one frame is the diamond case, and must collapse to a
/// single evaluation exactly as two component sources do.
#[test]
fn two_cells_changing_evaluate_a_node_once() {
    let mut app = app();
    let world = app.world_mut();

    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();

    let (left, right, node) = {
        let mut commands = world.commands();
        let left = commands.cell(1);
        let right = commands.cell(10);
        let node = {
            let (left, right) = (left.clone(), right.clone());
            commands.derive(move |s| {
                counter.fetch_add(1, Ordering::Relaxed);
                Ok(Sum(*left.read(s) + *right.read(s)))
            })
        };
        (left, right, node)
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(11)));

    let settled = runs.load(Ordering::Relaxed);
    left.set(2);
    right.set(20);
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(22)));
    assert_eq!(runs.load(Ordering::Relaxed), settled + 1);
}

/// A cell is level zero, so a chain hanging off one has to level and settle in
/// a single frame just like a chain off a component.
#[test]
fn a_chain_of_cell_readers_settles_in_one_frame() {
    let mut app = app();
    let world = app.world_mut();

    let (cell, first, second) = {
        let mut commands = world.commands();
        let cell = commands.cell(3);
        let first = commands.derive({
            let cell = cell.clone();
            move |s| Ok(Left(*cell.read(s) * 2))
        });
        let second = commands.derive({
            let first = first.clone();
            move |s| Ok(Bottom(first.get(s)?.0 + 1))
        });
        (cell, first, second)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(level(world, &first), 1);
    assert_eq!(level(world, &second), 2);
    assert_eq!(world.get::<Bottom>(second.entity()), Some(&Bottom(7)));

    cell.set(10);
    let passes = settle_reactive(world);
    assert_eq!(world.get::<Bottom>(second.entity()), Some(&Bottom(21)));
    assert!(passes <= 3, "the chain took {passes} passes to settle");
}

#[test]
fn an_effect_reads_a_cell() {
    let mut app = app();
    let world = app.world_mut();
    world.init_resource::<EffectLog>();

    let cell = {
        let mut commands = world.commands();
        let cell = commands.cell(3);
        commands.spawn(Effect::new({
            let cell = cell.clone();
            move |_: In<Entity>, store: SignalStore, mut log: ResMut<EffectLog>| {
                log.record(*cell.read(&store));
            }
        }));
        cell
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(effect_log(world).values, vec![3]);

    cell.set(4);
    run_reactive_schedule(world);
    assert_eq!(effect_log(world).values, vec![3, 4]);
}

/// Lazy tracking has to work the same way for cells: a branch that stops
/// reading one must stop being woken by it.
#[test]
fn a_node_drops_the_cell_edges_it_stops_reading() {
    let mut app = app();
    let world = app.world_mut();

    let read_left = Arc::new(AtomicBool::new(true));
    let branch = read_left.clone();
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();

    let (left, right, node) = {
        let mut commands = world.commands();
        let left = commands.cell(1);
        let right = commands.cell(100);
        let node = {
            let (left, right) = (left.clone(), right.clone());
            commands.derive(move |s| {
                counter.fetch_add(1, Ordering::Relaxed);
                let value = if branch.load(Ordering::Relaxed) {
                    *left.read(s)
                } else {
                    *right.read(s)
                };
                Ok(Sum(value))
            })
        };
        (left, right, node)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(1)));
    assert_eq!(cell_subscribers(world, left.entity()), 1);
    assert_eq!(cell_subscribers(world, right.entity()), 0);

    // Flip the branch, and poke the cell it is still reading to wake it.
    read_left.store(false, Ordering::Relaxed);
    left.set(2);
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(100)));
    assert_eq!(cell_subscribers(world, left.entity()), 0);
    assert_eq!(cell_subscribers(world, right.entity()), 1);

    let settled = runs.load(Ordering::Relaxed);
    left.set(3);
    run_reactive_schedule(world);
    assert_eq!(
        runs.load(Ordering::Relaxed),
        settled,
        "the abandoned cell must no longer wake the node"
    );

    right.set(200);
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(200)));
}

#[test]
fn a_despawned_node_unsubscribes_from_its_cell() {
    let mut app = app();
    let world = app.world_mut();

    let (cell, node) = {
        let mut commands = world.commands();
        let cell = commands.cell(1);
        let node = commands.derive({
            let cell = cell.clone();
            move |s| Ok(Sum(*cell.read(s)))
        });
        (cell, node)
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(cell_subscribers(world, cell.entity()), 1);

    world.despawn(node.entity());
    world.flush();
    assert_eq!(cell_subscribers(world, cell.entity()), 0);

    // And the cell must still be usable afterwards.
    cell.set(9);
    run_reactive_schedule(world);
    assert_eq!(*cell.peek(), 9);
}

/// The registry holds cells weakly, so dropping the last handle is what frees
/// the entity standing in for it.
#[test]
fn dropping_the_last_handle_collects_the_cell() {
    let mut app = app();
    let world = app.world_mut();

    // Something to write, since the sweep is gated on a write happening
    // somewhere — which is what keeps a quiet frame from walking the registry.
    let keepalive = world.commands().cell(0);

    let cell = world.commands().cell(1);
    let entity = cell.entity();
    let clone = cell.clone();
    world.flush();
    run_reactive_schedule(world);
    assert!(world.get_entity(entity).is_ok());

    drop(clone);
    keepalive.set(1);
    run_reactive_schedule(world);
    assert!(
        world.get_entity(entity).is_ok(),
        "a cell with a live handle must survive"
    );

    drop(cell);
    keepalive.set(2);
    run_reactive_schedule(world);
    assert!(world.get_entity(entity).is_err());
}

// ---------------------------------------------------------------------------
// Binding to the entity a bundle lands on
// ---------------------------------------------------------------------------

/// The ordinary shape: the watched component is on one entity, the mapped
/// output on another, and neither entity was known when the signal was built.
#[test]
fn a_watch_bundle_binds_to_the_entity_it_is_inserted_on() {
    let mut app = app();
    let world = app.world_mut();

    let target = {
        let mut commands = world.commands();
        let watch = commands.signal::<&Source>().watch_bundle();
        let target = commands.spawn(watch.map(double)).id();
        commands.spawn((Source(21), watch));
        target
    };
    world.flush();

    // No schedule run: binding is what lets the subscriber settle inline, the
    // same as `watch`.
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(42)));
}

/// The regression guard for the ordering hazard `Watching::UNBOUND` exists to
/// remove.
///
/// Every subscription path resolves `Watching` from inside a queued command,
/// while `WatchTarget` binds from a hook. If that binding were also queued, this
/// would pass in one order and silently drop the subscription in the other,
/// because hooks fire — and so queue their commands — in the bundle's component
/// order. Both orders must work.
#[test]
fn a_watch_bundle_binds_regardless_of_its_place_in_the_bundle() {
    for watch_first in [true, false] {
        let mut app = app();
        let world = app.world_mut();

        let host = {
            let mut commands = world.commands();
            let watch = commands.signal::<&Source>().watch_bundle();
            let mapped = watch.map(double);
            if watch_first {
                commands.spawn((Source(4), watch, mapped)).id()
            } else {
                commands.spawn((Source(4), mapped, watch)).id()
            }
        };
        world.flush();

        assert_eq!(
            world.get::<Doubled>(host),
            Some(&Doubled(8)),
            "a watch_bundle placed {} the mapped signal did not bind in time",
            if watch_first { "before" } else { "after" }
        );
    }
}

/// Binding is not just a one-shot initial evaluation: the host has to end up in
/// the scan's query the way a `watch`ed entity does.
#[test]
fn a_watch_bundle_host_dispatches_on_change() {
    let mut app = app();
    let world = app.world_mut();

    let (host, target) = {
        let mut commands = world.commands();
        let watch = commands.signal::<&Source>().watch_bundle();
        let target = commands.spawn(watch.map(double)).id();
        let host = commands.spawn((Source(1), watch)).id();
        (host, target)
    };
    world.flush();
    run_reactive_schedule(world);

    world.get_mut::<Source>(host).unwrap().0 = 5;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(10)));
    assert_eq!(subscribers::<&Source>(world, host), 1);
}

/// A signal entity carries `Watching` from the moment it is spawned, so "not
/// bound yet" has to be a value rather than a missing component — otherwise
/// every reader would take an unbound signal for a bound one.
#[test]
fn an_unbound_signal_reads_as_unwatched() {
    let mut app = app();
    let world = app.world_mut();

    let signal = {
        let mut commands = world.commands();
        commands.signal::<&Source>().watch_bundle()
    };
    world.flush();

    let watching = world
        .get::<Watching>(signal.entity())
        .expect("a signal entity carries `Watching` from its spawn");
    assert_eq!(watching.target(), None);

    // And a read through it fails as unwatched rather than resolving to the
    // placeholder entity.
    let unwatched = world
        .run_system_once(move |store: SignalStore| {
            matches!(signal.get(&store), Err(ReactError::NotWatched(_)))
        })
        .expect("the probe system should run");
    assert!(unwatched, "an unbound signal must not resolve to a target");
}

/// The component *is* the binding's lifetime, so losing it has to unbind the
/// signal rather than leave it reporting its old host's value.
#[test]
fn removing_a_watch_bundle_unbinds_the_signal() {
    let mut app = app();
    let world = app.world_mut();

    let (host, entity) = {
        let mut commands = world.commands();
        let watch = commands.signal::<&Source>().watch_bundle();
        let entity = watch.entity();
        let host = commands.spawn((Source(3), watch)).id();
        (host, entity)
    };
    world.flush();

    assert_eq!(world.get::<Watching>(entity).unwrap().target(), Some(host));

    world.entity_mut(host).remove::<WatchTarget<&Source>>();
    world.flush();

    assert_eq!(world.get::<Watching>(entity).unwrap().target(), None);
}

/// A subscriber that arrives before the binding parks itself rather than being
/// dropped, so the mapper system path survives a host spawned later.
#[test]
fn a_mapper_system_waits_for_a_late_binding() {
    let mut app = app();
    let world = app.world_mut();

    let (target, watch) = {
        let mut commands = world.commands();
        let watch = commands.signal::<&Source>().watch_bundle();
        let target = commands.spawn(watch.map_system(double_system)).id();
        (target, watch)
    };
    world.flush();

    assert_eq!(
        world.get::<Doubled>(target),
        None,
        "there is nothing to map until the signal is bound"
    );

    let host = world.spawn((Source(21), watch)).id();
    world.flush();

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(42)));
    assert_eq!(subscribers::<&Source>(world, host), 0, "no plain closures");
    assert_eq!(
        world
            .get::<SubscriberSet<&Source>>(host)
            .map_or(0, |set| set.system_len()),
        1
    );
}

/// The derived path has no retry of its own: a node whose read failed holds an
/// error and has no edge to be woken through, so parking is what keeps it from
/// being stuck for good.
#[test]
fn a_derived_node_waits_for_a_late_binding() {
    let mut app = app();
    let world = app.world_mut();

    let (node, watch) = {
        let mut commands = world.commands();
        let watch = commands.signal::<&Source>().watch_bundle();
        let signal = watch.signal();
        let node = commands.derive(move |store| Ok(Doubled(signal.get(store)?.0 * 2)));
        (node.entity(), watch)
    };
    world.flush();

    // Evaluates against an unbound signal and fails. This is the pass that has
    // to leave something behind for the binding to find.
    run_reactive_schedule(world);
    assert_eq!(world.get::<Doubled>(node), None);

    let host = world.spawn((Source(6), watch)).id();
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(node), Some(&Doubled(12)));

    // And it is genuinely subscribed now, not just evaluated once.
    world.get_mut::<Source>(host).unwrap().0 = 10;
    run_reactive_schedule(world);
    assert_eq!(world.get::<Doubled>(node), Some(&Doubled(20)));
}

// ---------------------------------------------------------------------------
// `Signal<T>`: one handle over every kind of source
// ---------------------------------------------------------------------------

/// A constant has nothing to subscribe to, and reporting a read anyway would
/// wire an edge that can never fire.
#[test]
fn a_value_signal_subscribes_to_nothing() {
    let mut app = app();
    let world = app.world_mut();

    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();

    let node = {
        let mut commands = world.commands();
        let signal = Signal::value(7);
        commands.derive(move |store| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(Sum(*signal.get(store)?))
        })
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(7)));
    assert_eq!(
        world.get::<NodeSources>(node.entity()).map(|s| s.0.len()),
        Some(0),
        "a constant must not become an edge"
    );

    let settled = runs.load(Ordering::Relaxed);
    run_reactive_schedule(world);
    assert_eq!(
        runs.load(Ordering::Relaxed),
        settled,
        "nothing can wake a node that only reads constants"
    );
}

/// The other three variants all have to reach the same graph the direct handles
/// do, so each is checked end to end: read, subscribe, wake.
#[test]
fn a_cell_signal_wakes_its_reader() {
    let mut app = app();
    let world = app.world_mut();

    let (cell, node) = {
        let mut commands = world.commands();
        let cell = commands.cell(1);
        let signal = Signal::from(cell.clone());
        let node = commands.derive(move |store| Ok(Sum(*signal.get(store)?)));
        (cell, node)
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(1)));

    cell.set(9);
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(9)));
}

#[test]
fn a_source_signal_wakes_its_reader() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(2)).id();

    let node = {
        let mut commands = world.commands();
        let signal = Signal::from(commands.signal::<&Source>().watch(source));
        commands.derive(move |store| Ok(Sum(signal.get(store)?.0)))
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(2)));

    world.get_mut::<Source>(source).unwrap().0 = 8;
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(8)));
}

/// Projection is what keeps `Signal<T>` usable for a `T` that cannot itself be
/// a component — `Entity` above all, which is what `watch_signal` binds on.
#[test]
fn a_projected_signal_reads_part_of_a_component() {
    let mut app = app();
    let world = app.world_mut();

    let first = world.spawn(Source(1)).id();
    let second = world.spawn(Source(2)).id();
    let holder = world.spawn(Selected(first)).id();

    let node = {
        let mut commands = world.commands();
        let selected = commands.signal::<&Selected>().watch(holder);
        let signal: Signal<Entity> = Signal::project(selected, |selected| &selected.0);
        commands.derive(move |store| Ok(Chosen(*signal.get(store)?)))
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Chosen>(node.entity()), Some(&Chosen(first)));

    world.get_mut::<Selected>(holder).unwrap().0 = second;
    run_reactive_schedule(world);
    assert_eq!(world.get::<Chosen>(node.entity()), Some(&Chosen(second)));
}

/// A derived signal is a `SourceSignal` over its output, so it needs no variant
/// of its own — this is the guard on that staying true.
#[test]
fn a_derived_signal_converts_without_a_variant_of_its_own() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(3)).id();

    let node = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let doubled = commands.derive(move |store| Ok(Doubled(signal.get(store)?.0 * 2)));

        let signal = Signal::from(doubled);
        commands.derive(move |store| Ok(Sum(signal.get(store)?.0 + 1)))
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(7)));

    world.get_mut::<Source>(source).unwrap().0 = 10;
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(21)));
}

/// The reactive/constant split is what lets a caller skip building machinery
/// that could never fire.
#[test]
fn a_signal_reports_whether_it_is_reactive() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(0)).id();

    let entity = world.spawn_empty().id();
    assert!(!Signal::from(entity).is_reactive());
    assert_eq!(Signal::from(entity).as_value(), Some(&entity));

    let mut commands = world.commands();
    assert!(Signal::from(commands.cell(0)).is_reactive());
    assert!(Signal::from(commands.signal::<&Source>().watch(source)).is_reactive());
}

// ---------------------------------------------------------------------------
// Following a reactive binding
// ---------------------------------------------------------------------------

/// The headline behaviour: a mapped subscriber follows the binding, and is
/// brought up to date against the new target rather than waiting for it to
/// change.
#[test]
fn a_mapped_signal_follows_a_rebound_signal() {
    let mut app = app();
    let world = app.world_mut();

    let first = world.spawn(Source(1)).id();
    let second = world.spawn(Source(2)).id();

    let (cell, target) = {
        let mut commands = world.commands();
        let cell = commands.cell(first);
        let signal = commands.signal::<&Source>().watch_signal(cell.clone());
        let target = commands.spawn(signal.map(double)).id();
        (cell, target)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(2)));
    assert_eq!(subscribers::<&Source>(world, first), 1);

    cell.set(second);
    run_reactive_schedule(world);

    // Neither source changed — only the binding did — so this value can only
    // come from the subscriber being dispatched against its new target.
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(4)));
    assert_eq!(
        subscribers::<&Source>(world, first),
        0,
        "the old target must not keep dispatching into the subscriber"
    );
    assert_eq!(subscribers::<&Source>(world, second), 1);

    // And the subscription really is live on the new target.
    world.get_mut::<Source>(second).unwrap().0 = 10;
    run_reactive_schedule(world);
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(20)));

    // ...while the old one is genuinely detached.
    world.get_mut::<Source>(first).unwrap().0 = 99;
    run_reactive_schedule(world);
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(20)));
}

/// A derived reader is re-evaluated rather than moved, so this checks the other
/// half of `repoint` — the edge is rewired and the node's value catches up.
#[test]
fn a_derived_node_follows_a_rebound_signal() {
    let mut app = app();
    let world = app.world_mut();

    let first = world.spawn(Source(1)).id();
    let second = world.spawn(Source(2)).id();

    let (cell, node) = {
        let mut commands = world.commands();
        let cell = commands.cell(first);
        let signal = commands.signal::<&Source>().watch_signal(cell.clone());
        let node = commands.derive(move |store| Ok(Sum(signal.get(store)?.0 * 10)));
        (cell, node)
    };
    world.flush();
    settle_reactive(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(10)));

    cell.set(second);
    settle_reactive(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(20)));

    // The edge moved with it: the new target wakes the node, the old one no
    // longer does.
    world.get_mut::<Source>(second).unwrap().0 = 7;
    settle_reactive(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(70)));

    world.get_mut::<Source>(first).unwrap().0 = 99;
    settle_reactive(world);
    assert_eq!(world.get::<Sum>(node.entity()), Some(&Sum(70)));
}

/// A source-backed binding, so the rebind is itself driven by a component
/// change rather than a cell write.
#[test]
fn a_signal_can_be_bound_by_another_signal() {
    let mut app = app();
    let world = app.world_mut();

    let first = world.spawn(Source(3)).id();
    let second = world.spawn(Source(4)).id();
    let holder = world.spawn(Selected(first)).id();

    let target = {
        let mut commands = world.commands();
        let selected = commands.signal::<&Selected>().watch(holder);
        let binding = Signal::project(selected, |selected| &selected.0);
        let signal = commands.signal::<&Source>().watch_signal(binding);
        commands.spawn(signal.map(double)).id()
    };
    world.flush();
    settle_reactive(world);
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(6)));

    world.get_mut::<Selected>(holder).unwrap().0 = second;
    settle_reactive(world);
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(8)));
}

/// A binding that cannot change must not build an effect, a node, or a system
/// registration — that is the whole reason `watch` stays a separate path.
#[test]
fn a_constant_binding_builds_no_machinery() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(21)).id();

    let (target, signal) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch_signal(source);
        let target = commands.spawn(signal.map(double)).id();
        (target, signal)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(42)));

    let signal = signal.entity();
    assert!(
        world.get::<Effect>(signal).is_none(),
        "a constant binding must not park an effect on the signal"
    );
    assert!(world.get::<NodeSources>(signal).is_none());
    assert!(world.get::<NodeLevel>(signal).is_none());
}

/// Unbinding parks the subscribers rather than dropping them, so a
/// `watch_bundle` moved from one host to another keeps working.
#[test]
fn a_watch_bundle_can_be_moved_between_hosts() {
    let mut app = app();
    let world = app.world_mut();

    let (target, watch) = {
        let mut commands = world.commands();
        let watch = commands.signal::<&Source>().watch_bundle();
        let target = commands.spawn(watch.map(double)).id();
        (target, watch)
    };
    world.flush();

    let first = world.spawn((Source(5), watch.clone())).id();
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(10)));

    // Move it: the old host loses the component, a new one gains it.
    world.entity_mut(first).remove::<WatchTarget<&Source>>();
    let second = world.spawn((Source(8), watch)).id();
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(16)));
    assert_eq!(subscribers::<&Source>(world, first), 0);
    assert_eq!(subscribers::<&Source>(world, second), 1);
}

/// Rebinding to the entity a signal is already on is a no-op, not a round trip
/// through detach and reattach.
#[test]
fn rebinding_to_the_same_entity_dispatches_nothing() {
    let mut app = app();
    let world = app.world_mut();

    let source = world.spawn(Source(1)).id();
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();

    let cell = {
        let mut commands = world.commands();
        let cell = commands.cell(source);
        let signal = commands.signal::<&Source>().watch_signal(cell.clone());
        commands.spawn(signal.map_system(move |ctx: SignalCtx<&Source>| {
            counter.fetch_add(1, Ordering::Relaxed);
            Doubled(ctx.data.0 * 2)
        }));
        cell
    };
    world.flush();
    settle_reactive(world);

    let settled = runs.load(Ordering::Relaxed);
    cell.set(source);
    settle_reactive(world);

    assert_eq!(
        runs.load(Ordering::Relaxed),
        settled,
        "writing the same entity back must not re-dispatch"
    );
}

// -- capturing mappers -------------------------------------------------------

/// `map` cannot capture, by design — that is what makes a mapper shareable.
/// `map_fn` is the escape hatch, and it has to behave identically in every
/// other respect.
#[test]
fn a_capturing_mapper_maps_on_insertion_and_on_change() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let offset = 100;
    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands
            .spawn(signal.map_fn(move |value: &Source| Doubled(value.0 * 2 + offset)))
            .id()
    };
    world.flush();

    // No schedule run: a capturing mapper owes the same on-insertion dispatch
    // a plain one does.
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(102)));

    run_reactive_schedule(world);
    world.get_mut::<Source>(source).unwrap().0 = 5;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(110)));
}

/// One `MappedSignal` handed to several entities shares the boxed closure, so
/// each has to get its own subscription rather than the first one winning.
#[test]
fn a_capturing_mapper_can_drive_several_entities() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(2)).id();

    let (first, second) = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let mapped = signal.map_fn(move |value: &Source| Doubled(value.0 * 3));
        let first = commands.spawn(mapped.clone()).id();
        let second = commands.spawn(mapped).id();
        (first, second)
    };
    world.flush();
    run_reactive_schedule(world);

    world.get_mut::<Source>(source).unwrap().0 = 4;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(first), Some(&Doubled(12)));
    assert_eq!(world.get::<Doubled>(second), Some(&Doubled(12)));
}

/// A capturing mapper is not a system, so it must not leave a registration
/// behind the way `map_system` does.
#[test]
fn a_capturing_mapper_registers_no_system() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let before = world
        .run_system_once(|systems: Query<&SystemIdMarker>| systems.iter().count())
        .unwrap();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.spawn(signal.map_fn(move |value: &Source| Doubled(value.0)));
    }
    world.flush();
    run_reactive_schedule(world);

    let after = world
        .run_system_once(|systems: Query<&SystemIdMarker>| systems.iter().count())
        .unwrap();
    assert_eq!(before, after);
}

// -- optional sources --------------------------------------------------------

/// What an `Option<&T>` mapper writes when `T` is absent. A distinct component
/// so the tests can tell "mapped to the absent form" from "never mapped".
#[derive(Component, Clone, Copy, Debug, PartialEq)]
struct Presence(bool);

fn presence(source: Option<&Source>) -> Presence {
    Presence(source.is_some())
}

/// A subscriber over `&T` never runs while `T` is missing. Over `Option<&T>`
/// absence is a value, so it runs immediately with `None`.
#[test]
fn an_optional_source_maps_when_the_component_is_absent() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn_empty().id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<Option<&Source>>().watch(source);
        commands.spawn(signal.map(presence)).id()
    };
    world.flush();

    assert_eq!(world.get::<Presence>(target), Some(&Presence(false)));
}

/// The whole point of the tier: `Changed<T>` cannot match a row that no longer
/// has `T`, so without the removal observer this would keep reporting `true`
/// forever.
#[test]
fn an_optional_source_dispatches_when_the_component_is_removed() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<Option<&Source>>().watch(source);
        commands.spawn(signal.map(presence)).id()
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Presence>(target), Some(&Presence(true)));

    world.entity_mut(source).remove::<Source>();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Presence>(target), Some(&Presence(false)));
}

/// Adding the component back is an ordinary `Changed` dispatch, but only if the
/// removal path left the subscription intact on the way out.
#[test]
fn an_optional_source_recovers_when_the_component_returns() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<Option<&Source>>().watch(source);
        commands.spawn(signal.map(presence)).id()
    };
    world.flush();
    run_reactive_schedule(world);

    world.entity_mut(source).remove::<Source>();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Presence>(target), Some(&Presence(false)));

    world.entity_mut(source).insert(Source(7));
    run_reactive_schedule(world);

    assert_eq!(world.get::<Presence>(target), Some(&Presence(true)));
}

/// A derived node subscribes by being *marked*, not by being run. A removal
/// dispatched with a throwaway `ReactiveWork` would drop that mark on the
/// floor, so the mark has to land in the pass's own.
#[test]
fn a_removal_wakes_a_derived_node() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(3)).id();

    let derived = {
        let mut commands = world.commands();
        let signal = commands.signal::<Option<&Source>>().watch(source);
        commands.derive(move |store| Ok(Sum(signal.get(store)?.map_or(-1, |s| s.0))))
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(derived.entity()), Some(&Sum(3)));

    world.entity_mut(source).remove::<Source>();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(derived.entity()), Some(&Sum(-1)));
}

/// A removal is drained at the top of a pass, beside the cell sweep, so it
/// settles within the frame it happened rather than the next one.
#[test]
fn a_removal_settles_in_one_frame() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<Option<&Source>>().watch(source);
        commands.spawn(signal.map(presence)).id()
    };
    world.flush();
    settle_reactive(world);

    world.entity_mut(source).remove::<Source>();
    let passes = settle_reactive(world);

    assert_eq!(world.get::<Presence>(target), Some(&Presence(false)));
    assert_eq!(passes, 2, "one pass to dispatch, one to prove it settled");
}

/// `Remove` also fires on despawn, at which point the subscriber set is going
/// away with the entity. There is nothing to dispatch and nothing to panic
/// about.
#[test]
fn despawning_an_optional_source_is_not_an_error() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<Option<&Source>>().watch(source);
        commands.spawn(signal.map(presence)).id()
    };
    world.flush();
    run_reactive_schedule(world);

    world.despawn(source);
    run_reactive_schedule(world);

    // The last value stands: there is no source left to report absence from.
    assert_eq!(world.get::<Presence>(target), Some(&Presence(true)));
}

/// A tuple's subscribers live in one `SubscriberSet` keyed by the whole tuple,
/// so a wakeup registered by the optional half has to dispatch the tuple rather
/// than the half.
#[test]
fn a_removal_wakes_a_tuple_source() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn((Source(2), Other(10))).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<(Option<&Source>, &Other)>().watch(source);
        commands
            .spawn(signal.map(|(source, other): (Option<&Source>, &Other)| {
                Sum(source.map_or(0, |s| s.0) + other.0)
            }))
            .id()
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(target), Some(&Sum(12)));

    world.entity_mut(source).remove::<Source>();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(target), Some(&Sum(10)));
}

/// A mapper system reads the same absent value a mapper closure does, and is
/// dispatched by the same removal path.
#[test]
fn a_removal_wakes_a_mapper_system() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<Option<&Source>>().watch(source);
        commands
            .spawn(
                signal.map_system(|ctx: SignalCtx<Option<&Source>>| Presence(ctx.data.is_some())),
            )
            .id()
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Presence>(target), Some(&Presence(true)));

    world.entity_mut(source).remove::<Source>();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Presence>(target), Some(&Presence(false)));
}

/// A relationship collection is *removed* when its last member leaves rather
/// than left empty, which is the case this tier exists for.
#[test]
fn an_emptied_relationship_collection_reports_absence() {
    let mut app = app();
    let world = app.world_mut();
    let container = world.spawn_empty().id();
    let child = world.spawn(ChildOf(container)).id();

    let target =
        {
            let mut commands = world.commands();
            let signal = commands.signal::<Option<&Children>>().watch(container);
            commands
                .spawn(signal.map(|children: Option<&Children>| {
                    Sum(children.map_or(-1, |c| c.len() as i32))
                }))
                .id()
        };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(target), Some(&Sum(1)));

    world.despawn(child);
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(target), Some(&Sum(-1)));
}

/// One observer per (component, signal) pair, however many signals are built.
#[test]
fn removal_observers_are_registered_once() {
    let mut app = app();
    let world = app.world_mut();

    let observers = |world: &mut World| {
        world
            .run_system_once(|observers: Query<&Observer>| observers.iter().count())
            .unwrap()
    };

    let before = observers(world);
    {
        let mut commands = world.commands();
        for _ in 0..5 {
            let source = commands.spawn(Source(1)).id();
            let signal = commands.signal::<Option<&Source>>().watch(source);
            commands.spawn(signal.map(presence));
        }
    }
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(observers(world), before + 1);
}

// -- resource signals --------------------------------------------------------

#[derive(Resource, Clone, Debug, PartialEq)]
struct Config {
    scale: i32,
    name: &'static str,
}

#[test]
fn a_resource_signal_reads_the_current_value() {
    let mut app = app();
    let world = app.world_mut();
    world.insert_resource(Config {
        scale: 3,
        name: "a",
    });

    let signal = {
        let mut commands = world.commands();
        commands.resource::<Config>()
    };
    let derived = {
        let signal = signal.clone();
        let mut commands = world.commands();
        commands.derive(move |store| Ok(Sum(signal.read(store).as_ref().map_or(0, |c| c.scale))))
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(derived.entity()), Some(&Sum(3)));
}

#[test]
fn a_resource_signal_wakes_readers_when_the_resource_changes() {
    let mut app = app();
    let world = app.world_mut();
    world.insert_resource(Config {
        scale: 3,
        name: "a",
    });

    let signal = {
        let mut commands = world.commands();
        commands.resource::<Config>()
    };
    let derived = {
        let signal = signal.clone();
        let mut commands = world.commands();
        commands.derive(move |store| Ok(Sum(signal.read(store).as_ref().map_or(0, |c| c.scale))))
    };
    world.flush();
    run_reactive_schedule(world);

    world.resource_mut::<Config>().scale = 10;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(derived.entity()), Some(&Sum(10)));
}

/// A resource the app has not inserted yet reads `None`, and starts reporting
/// the moment it appears — the scanner is what notices, so this must not depend
/// on the signal being built after the resource.
#[test]
fn a_resource_signal_picks_up_a_late_resource() {
    let mut app = app();
    let world = app.world_mut();

    let signal = {
        let mut commands = world.commands();
        commands.resource::<Config>()
    };
    let derived = {
        let signal = signal.clone();
        let mut commands = world.commands();
        commands.derive(move |store| Ok(Sum(signal.read(store).as_ref().map_or(-1, |c| c.scale))))
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Sum>(derived.entity()), Some(&Sum(-1)));

    world.insert_resource(Config {
        scale: 5,
        name: "a",
    });
    run_reactive_schedule(world);

    assert_eq!(world.get::<Sum>(derived.entity()), Some(&Sum(5)));
}

/// A frame in which the resource did not change costs one `is_changed` check
/// and wakes nobody.
#[test]
fn an_unchanged_resource_wakes_nobody() {
    let mut app = app();
    let world = app.world_mut();
    world.insert_resource(Config {
        scale: 3,
        name: "a",
    });

    let runs = Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();
    let signal = {
        let mut commands = world.commands();
        commands.resource::<Config>()
    };
    {
        let signal = signal.clone();
        let mut commands = world.commands();
        commands.derive(move |store| {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(Sum(signal.read(store).as_ref().map_or(0, |c| c.scale)))
        });
    }
    world.flush();
    run_reactive_schedule(world);

    let settled = runs.load(Ordering::Relaxed);
    run_reactive_schedule(world);
    run_reactive_schedule(world);

    assert_eq!(runs.load(Ordering::Relaxed), settled);
}

/// A projection narrows what gets copied out of the resource. It does not
/// narrow what wakes the reader — that is the resource's tick — which is
/// exactly what the doc comment promises.
#[test]
fn a_projected_resource_signal_copies_only_the_projection() {
    let mut app = app();
    let world = app.world_mut();
    world.insert_resource(Config {
        scale: 3,
        name: "a",
    });

    let signal = {
        let mut commands = world.commands();
        commands.resource_with(|config: &Config| config.name)
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(signal.peek_cloned(), Some("a"));

    world.resource_mut::<Config>().name = "b";
    run_reactive_schedule(world);

    assert_eq!(signal.peek_cloned(), Some("b"));
}

/// The scanner's registry holds weak handles, so a signal nobody kept stops
/// costing anything rather than being written forever.
#[test]
fn dropping_the_last_resource_handle_prunes_the_scanner() {
    let mut app = app();
    let world = app.world_mut();
    world.insert_resource(Config {
        scale: 3,
        name: "a",
    });

    let signal = {
        let mut commands = world.commands();
        commands.resource::<Config>()
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(crate::signal2::resource::watching_count::<Config>(world), 1);

    drop(signal);
    // The scanner learns the handle is gone the next time it tries to write.
    world.resource_mut::<Config>().scale = 4;
    run_reactive_schedule(world);

    assert_eq!(
        crate::signal2::resource::watching_count::<Config>(world),
        0,
        "a dropped signal must stop costing a clone per change"
    );
}

// -- derived values ----------------------------------------------------------

/// The wrapper exists so a derived signal can produce something that is not a
/// component; reading it back must not make the caller think about that.
#[test]
fn derive_value_wraps_a_plain_value() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(4)).id();

    let derived = {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        commands.derive_value(move |store| Ok(signal.get(store)?.0 > 2))
    };
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(
        world.get::<Derived<bool>>(derived.entity()),
        Some(&Derived(true))
    );

    world.get_mut::<Source>(source).unwrap().0 = 1;
    run_reactive_schedule(world);

    assert_eq!(
        world.get::<Derived<bool>>(derived.entity()),
        Some(&Derived(false))
    );
}

/// A wrapped value reads back through `project` as a plain `Signal<T>`, which
/// is what makes a derived `Entity` usable as a binding.
#[test]
fn a_derived_value_projects_to_a_signal() {
    let mut app = app();
    let world = app.world_mut();
    let first = world.spawn(Source(1)).id();
    let second = world.spawn(Source(2)).id();
    let chooser = world.spawn(Selected(first)).id();

    let target = {
        let mut commands = world.commands();
        let selected = commands.signal::<&Selected>().watch(chooser);
        let chosen = commands.derive_value(move |store| Ok(selected.get(store)?.0));
        let signal = commands.signal::<&Source>().watch_signal(chosen.project());
        commands.spawn(signal.map(double)).id()
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(2)));

    world.get_mut::<Selected>(chooser).unwrap().0 = second;
    run_reactive_schedule(world);

    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(4)));
}

/// `derive_list` is the wrapper plus the blanket `ListSource`: a computed `Vec`
/// drives a list with no collection component in sight.
#[test]
fn derive_list_builds_rows_from_a_computed_collection() {
    let mut app = app();
    let world = app.world_mut();
    let source = world.spawn(Source(3)).id();
    let container = world.spawn_empty().id();

    {
        let mut commands = world.commands();
        let signal = commands.signal::<&Source>().watch(source);
        let list: ReactiveList = commands.derive_list(
            move |store| Ok((0..signal.get(store)?.0 as u32).collect::<Vec<u32>>()),
            |item| *item,
            |item, _| RowValue(*item),
        );
        commands.entity(container).insert(list);
    }
    world.flush();
    run_reactive_schedule(world);

    assert_eq!(row_values(world, container), vec![0, 1, 2]);

    world.get_mut::<Source>(source).unwrap().0 = 5;
    run_reactive_schedule(world);

    assert_eq!(row_values(world, container), vec![0, 1, 2, 3, 4]);
}

// -- conditional rendering ---------------------------------------------------

/// v1 spelled "render this when the value is there, and take it back down when
/// it isn't" as `.option().map(..)`. v2 has no such combinator and does not need
/// one: [`AnyBundle`](crate::any::AnyBundle) already erases a bundle behind a
/// component whose `on_replace` removes whatever the last one inserted, so the
/// absent arm is just `().into_any()`.
///
/// This is the shape the whole view layer is built out of, so it is worth
/// pinning that the two tiers compose rather than assuming it.
///
/// Note the plugin set. `AnyBundle`'s teardown goes through the v1
/// [`CleanupRegistry`](crate::cleanup::CleanupRegistry), which
/// [`ReactivePlugin`] does not install — so signal2 on its own can map *to* an
/// `AnyBundle` but panics when one is replaced. An app running both tiers (as
/// one mid-migration does) gets it from `ReactPlugin`.
#[test]
fn an_optional_source_can_add_and_remove_a_bundle() {
    use crate::any::{AnyBundle, IntoAnyBundle};

    let mut app = App::new();
    app.add_plugins((ReactivePlugin, crate::cleanup::CleanupPlugin));
    let world = app.world_mut();
    let source = world.spawn(Source(1)).id();

    let target = {
        let mut commands = world.commands();
        let signal = commands.signal::<Option<&Source>>().watch(source);
        commands
            .spawn(signal.map(|value: Option<&Source>| -> AnyBundle {
                match value {
                    Some(value) => Doubled(value.0 * 2).into_any(),
                    None => ().into_any(),
                }
            }))
            .id()
    };
    world.flush();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(2)));

    // Removing the source takes the rendered bundle back off.
    world.entity_mut(source).remove::<Source>();
    run_reactive_schedule(world);
    assert_eq!(world.get::<Doubled>(target), None);

    // And putting it back renders it again.
    world.entity_mut(source).insert(Source(4));
    run_reactive_schedule(world);
    assert_eq!(world.get::<Doubled>(target), Some(&Doubled(8)));
}
