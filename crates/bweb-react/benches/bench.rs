//! Reactive-runtime benchmarks comparing the two signal implementations:
//!
//! - `signal` — the original pull/`Changed`-detection runtime (`bweb_react::signal`,
//!   wired by [`ReactPlugin`]). Derived systems read ECS directly via `SQuery`
//!   and re-run when a queried component is `Changed`.
//! - `signal2` — the newer push-based graph (`bweb_react::signal2`, wired by
//!   `Signal2Plugin`). External change enters through a query-observer *input*
//!   signal; derived signals read *other signals* and are marked dirty by an
//!   explicit push through the dependency graph.
//!
//! Because the two runtimes are driven differently, each group builds an
//! equivalent graph in both and drives it the natural way for that runtime:
//! `signal` mutates a component (`get_mut`, tripping `Changed`); `signal2`
//! re-inserts the component, which re-fires the input's query observer (a plain
//! mutation emits no lifecycle event and would not propagate). Both then
//! propagate one change out to the same fan-out / chain shape.

use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use bweb_react::prelude::*;
use bweb_react::signal::signal_world::SWorld;
use criterion::{
    AxisScale, BenchmarkId, Criterion, PlotConfiguration, criterion_group, criterion_main,
};
use std::hint::black_box;

#[derive(Component, PartialEq, Clone)]
struct TestData<T>(pub T);

/// `1, 10, 100, 1_000, 10_000` — the fan-out / chain sizes swept by every group.
fn sizes() -> impl Iterator<Item = usize> {
    (0..5).map(|i| 10usize.pow(i))
}

/// Push-based `signal2` graph setups.
///
/// Kept in a submodule so the `signal2` `Signal`/`SignalRead` traits resolve
/// against `Commands` without colliding with the old `SignalExt`/`Signal` in the
/// parent scope (both define `derive`/`memo`).
mod v2 {
    use super::TestData;
    use bevy_app::prelude::*;
    use bevy_ecs::{prelude::*, system::SystemId};
    use bevy_query_observer::Start;
    use bweb_react::signal2::{ObserverSignal, Signal2Plugin, SignalExt, SignalRead};

    pub fn app() -> App {
        let mut app = App::new();
        app.add_plugins(Signal2Plugin);
        app
    }

    /// An observer input over `source`, plus `count` derived signals that each
    /// read it. Fully built and settled. Returns the app and the source entity.
    pub fn fanout(count: usize) -> (App, Entity) {
        let mut app = app();
        let source;
        {
            let world = app.world_mut();
            source = world.spawn(TestData(1f32)).id();
            {
                let mut commands = world.commands();
                let input = commands
                    .signal(|c: Start<&TestData<f32>>| c.0)
                    .watch_entity(source);
                for _ in 0..count {
                    let input = input.clone();
                    commands.derive(move || Ok(input.get()?));
                }
            }
            world.flush();
        }
        app.update();
        (app, source)
    }

    /// Re-inserts a component value, re-firing the source's query observer — the
    /// `signal2` analogue of the `signal` benches' `get_mut` mutation.
    pub fn drive<T: Clone + Send + Sync + 'static>(app: &mut App, entity: Entity, value: T) {
        app.world_mut().entity_mut(entity).insert(TestData(value));
    }

    /// One source of `value` fanning out to `count` derived signals. Queues its
    /// commands onto `world`; the caller flushes.
    fn source<T: Clone + Send + Sync + 'static>(
        world: &mut World,
        value: T,
        count: usize,
    ) -> Entity {
        let entity = world.spawn(TestData(value)).id();
        let mut commands = world.commands();
        let input = commands
            .signal(move |c: Start<&TestData<T>>| c.0.clone())
            .watch_entity(entity);
        for _ in 0..count {
            let input = input.clone();
            commands.derive(move || Ok(input.get()?));
        }
        entity
    }

    /// Four independent sources (f32/i32/String/bool), each fanning out to
    /// `count` derived signals. Returns the app and the four source entities.
    pub fn multi(count: usize) -> (App, [Entity; 4]) {
        let mut app = app();
        let sources;
        {
            let world = app.world_mut();
            sources = [
                source(world, 1f32, count),
                source(world, 1i32, count),
                source(world, uuid::Uuid::new_v4().simple().to_string(), count),
                source(world, false, count),
            ];
            world.flush();
        }
        app.update();
        (app, sources)
    }

    /// `chains` independent linear chains of depth 6 (an observer input + 5
    /// stacked derives), plus a one-shot system that re-inserts every source to
    /// drive a full propagation down every chain.
    pub fn serial_chain(chains: usize) -> (App, SystemId) {
        let mut app = app();
        let updater;
        {
            let world = app.world_mut();
            {
                let mut commands = world.commands();
                for _ in 0..chains {
                    let src = commands.spawn(TestData(1f32)).id();
                    let input = commands
                        .signal(|c: Start<&TestData<f32>>| c.0)
                        .watch_entity(src);
                    let mut sig = commands.derive({
                        let input = input.clone();
                        move || Ok(input.get()?)
                    });
                    for _ in 0..5 {
                        let prev = sig.clone();
                        sig = commands.derive(move || Ok(prev.get()? + 1.0));
                    }
                }
            }
            updater =
                world.register_system(|q: Query<(Entity, &TestData<f32>)>, mut c: Commands| {
                    for (e, d) in &q {
                        c.entity(e).insert(TestData(d.0 + 1.0));
                    }
                });
            world.flush();
        }
        app.update();
        (app, updater)
    }

    /// An unflushed observer input over `source`, for the spawn-speed workload to
    /// give freshly-created derives something to read.
    pub fn observer_input(app: &mut App, source: Entity) -> ObserverSignal<f32> {
        let world = app.world_mut();
        let mut commands = world.commands();
        commands
            .signal(|c: Start<&TestData<f32>>| c.0)
            .watch_entity(source)
    }

    /// Queues `count` derives reading `input` (no flush) — the spawn-speed
    /// workload.
    pub fn queue_derives(app: &mut App, input: &ObserverSignal<f32>, count: usize) {
        let world = app.world_mut();
        let mut commands = world.commands();
        for _ in 0..count {
            let input = input.clone();
            commands.derive(move || Ok(input.get()?));
        }
    }
}

/// Builds a `signal` (original runtime) app with a single `TestData<f32>` source
/// entity fanned out to `count` derived signals reading it via `SQuery`.
fn signal_fanout(count: usize) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(ReactPlugin);
    let world = app.world_mut();

    let test_entity = world.spawn(TestData(1f32)).id();
    for _ in 0..count {
        let mut commands = world.commands();
        let derived = commands
            .derive(move |data: SQuery<&TestData<f32>>| data.get(test_entity).unwrap().clone());
        commands.spawn(derived);
    }
    (app, test_entity)
}

fn observer_benches(c: &mut Criterion) {
    // ---- Fan-out, driven every iteration -----------------------------------
    let mut group = c.benchmark_group("run with update");
    for i in sizes() {
        group.bench_with_input(BenchmarkId::new("signal", i), &i, |b, i| {
            let (mut app, test_entity) = signal_fanout(*i);
            b.iter(|| {
                app.world_mut()
                    .get_mut::<TestData<f32>>(test_entity)
                    .unwrap()
                    .0 += 0.1;
                app.update();
                black_box(&mut app);
            });
        });
        group.bench_with_input(BenchmarkId::new("signal2", i), &i, |b, i| {
            let (mut app, source) = v2::fanout(*i);
            let mut value = 1f32;
            b.iter(|| {
                value += 0.1;
                v2::drive(&mut app, source, value);
                app.update();
                black_box(&mut app);
            });
        });
    }
    group.finish();

    // ---- Fan-out, idle update (no change pending) --------------------------
    let mut group = c.benchmark_group("run without update");
    for i in sizes() {
        group.bench_with_input(BenchmarkId::new("signal", i), &i, |b, i| {
            let (mut app, test_entity) = signal_fanout(*i);
            app.world_mut()
                .get_mut::<TestData<f32>>(test_entity)
                .unwrap()
                .0 += 0.1;
            app.update();

            b.iter(|| {
                app.update();
                black_box(&mut app);
            });
        });
        group.bench_with_input(BenchmarkId::new("signal2", i), &i, |b, i| {
            let (mut app, source) = v2::fanout(*i);
            v2::drive(&mut app, source, 2f32);
            app.update();

            b.iter(|| {
                app.update();
                black_box(&mut app);
            });
        });
    }
    group.finish();

    // ---- Four independent sources, driven every iteration ------------------
    let mut group = c.benchmark_group("multiple sources");
    for i in sizes() {
        group.bench_with_input(BenchmarkId::new("signal", i), &i, |b, i| {
            let mut app = App::new();
            app.add_plugins(ReactPlugin);
            let world = app.world_mut();

            fn spawn_signals<T: Clone + Send + Sync + 'static>(
                value: T,
                total: usize,
                world: &mut World,
            ) -> Entity {
                let entity = world.spawn(TestData(value)).id();
                for _ in 0..total {
                    let mut commands = world.commands();
                    let derived = commands.derive(move |data: SQuery<&TestData<T>>| {
                        data.get(entity).unwrap().clone()
                    });
                    commands.spawn(derived);
                }
                entity
            }

            let float = spawn_signals(1f32, *i, world);
            let int = spawn_signals(1i32, *i, world);
            let string = spawn_signals(uuid::Uuid::new_v4().simple().to_string(), *i, world);
            let bool = spawn_signals(false, *i, world);

            b.iter(|| {
                app.world_mut().get_mut::<TestData<f32>>(float).unwrap().0 += 0.1;
                app.world_mut().get_mut::<TestData<i32>>(int).unwrap().0 += 1;
                app.world_mut()
                    .get_mut::<TestData<String>>(string)
                    .unwrap()
                    .0 = uuid::Uuid::new_v4().simple().to_string();

                let val = app.world_mut().get_mut::<TestData<bool>>(bool).unwrap().0;
                app.world_mut().get_mut::<TestData<bool>>(bool).unwrap().0 = !val;

                app.update();
                black_box(&mut app);
            });
        });
        group.bench_with_input(BenchmarkId::new("signal2", i), &i, |b, i| {
            let (mut app, [float, int, string, boolean]) = v2::multi(*i);
            let mut f = 1f32;
            let mut n = 1i32;
            let mut flag = false;
            b.iter(|| {
                f += 0.1;
                n += 1;
                flag = !flag;
                v2::drive(&mut app, float, f);
                v2::drive(&mut app, int, n);
                v2::drive(&mut app, string, uuid::Uuid::new_v4().simple().to_string());
                v2::drive(&mut app, boolean, flag);
                app.update();
                black_box(&mut app);
            });
        });
    }
    group.finish();

    // ---- Four independent sources, idle update -----------------------------
    let mut group = c.benchmark_group("multiple sources without update");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));
    for i in sizes() {
        group.bench_with_input(BenchmarkId::new("signal", i), &i, |b, i| {
            let mut app = App::new();
            app.add_plugins(ReactPlugin);
            let world = app.world_mut();

            fn spawn_signals<T: Clone + Send + Sync + 'static>(
                value: T,
                total: usize,
                world: &mut World,
            ) -> Entity {
                let entity = world.spawn(TestData(value)).id();
                for _ in 0..total {
                    let mut commands = world.commands();
                    let derived = commands.derive(move |data: SQuery<&TestData<T>>| {
                        data.get(entity).unwrap().clone()
                    });
                    commands.spawn(derived);
                }
                entity
            }

            let float = spawn_signals(1f32, *i, world);
            let int = spawn_signals(1i32, *i, world);
            let string = spawn_signals(uuid::Uuid::new_v4().simple().to_string(), *i, world);
            let bool = spawn_signals(false, *i, world);

            app.world_mut().get_mut::<TestData<f32>>(float).unwrap().0 += 0.1;
            app.world_mut().get_mut::<TestData<i32>>(int).unwrap().0 += 1;
            app.world_mut()
                .get_mut::<TestData<String>>(string)
                .unwrap()
                .0 = uuid::Uuid::new_v4().simple().to_string();

            let val = app.world_mut().get_mut::<TestData<bool>>(bool).unwrap().0;
            app.world_mut().get_mut::<TestData<bool>>(bool).unwrap().0 = !val;
            app.update();

            b.iter(|| {
                app.update();
                black_box(&mut app);
            });
        });
        group.bench_with_input(BenchmarkId::new("signal2", i), &i, |b, i| {
            let (mut app, [float, int, string, boolean]) = v2::multi(*i);
            v2::drive(&mut app, float, 2f32);
            v2::drive(&mut app, int, 2i32);
            v2::drive(&mut app, string, uuid::Uuid::new_v4().simple().to_string());
            v2::drive(&mut app, boolean, true);
            app.update();

            b.iter(|| {
                app.update();
                black_box(&mut app);
            });
        });
    }
    group.finish();

    // ---- `signal`-internal: SWorld vs SQuery read params (idle re-eval) -----
    // No `signal2` analogue: derives read signal handles, not ECS params, and an
    // idle `signal2` flush does no work.
    let mut group = c.benchmark_group("get speed");
    group.bench_with_input("world", &1000, |b, i| {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        let test_entity = world.spawn(TestData(1f32)).id();

        for _ in 0..*i {
            let mut commands = world.commands();
            let derived = commands
                .memo(move |data: SWorld| data.get::<TestData<f32>>(test_entity).unwrap().clone());
            commands.spawn(derived);
        }

        b.iter(|| {
            app.update();
            black_box(&mut app);
        });
    });
    group.bench_with_input("query", &1000, |b, i| {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        let test_entity = world.spawn(TestData(1f32)).id();

        for _ in 0..*i {
            let mut commands = world.commands();
            let derived = commands
                .memo(move |data: SQuery<&TestData<f32>>| data.get(test_entity).unwrap().clone());
            commands.spawn(derived);
        }

        b.iter(|| {
            app.update();
            black_box(&mut app);
        });
    });
    group.finish();

    // ---- Node-creation cost -------------------------------------------------
    let mut group = c.benchmark_group("spawn speed");
    group.bench_with_input("signal (world)", &1000, |b, i| {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        let test_entity = world.spawn(TestData(1f32)).id();

        b.iter(|| {
            let world = app.world_mut();
            let mut commands = world.commands();
            for _ in 0..*i {
                let derived = commands.memo(move |data: SWorld| {
                    data.get::<TestData<f32>>(test_entity).unwrap().clone()
                });
                commands.spawn(derived);
            }
            black_box(&mut app);
        });
    });
    group.bench_with_input("signal (query)", &1000, |b, i| {
        let mut app = App::new();
        app.add_plugins(ReactPlugin);
        let world = app.world_mut();

        let test_entity = world.spawn(TestData(1f32)).id();

        b.iter(|| {
            let world = app.world_mut();
            let mut commands = world.commands();
            for _ in 0..*i {
                let derived = commands.memo(move |data: SQuery<&TestData<f32>>| {
                    data.get(test_entity).unwrap().clone()
                });
                commands.spawn(derived);
            }
            black_box(&mut app);
        });
    });
    group.bench_with_input("signal2", &1000, |b, i| {
        let mut app = v2::app();
        let source = app.world_mut().spawn(TestData(1f32)).id();
        let input = v2::observer_input(&mut app, source);

        b.iter(|| {
            v2::queue_derives(&mut app, &input, *i);
            black_box(&mut app);
        });
    });
    group.finish();

    // ---- Serial chains (depth 6), fully propagated every iteration ---------
    let mut group = c.benchmark_group("serial chain");
    for i in sizes() {
        group.bench_with_input(BenchmarkId::new("signal", i), &i, |b, i| {
            let mut app = App::new();
            app.add_plugins(ReactPlugin);
            let world = app.world_mut();

            let mut commands = world.commands();
            for _ in 0..*i {
                let test_entity = commands.spawn(TestData(1f32)).id();

                let mut sig = commands.derive(move |value: SQuery<&TestData<f32>>| {
                    value.get(test_entity).unwrap().clone()
                });

                for _ in 0..5 {
                    let new_sig = commands.derive({
                        let sig = sig.clone();
                        move || TestData(sig.get().0 + 1.0)
                    });
                    sig = new_sig;
                }
            }

            let updater = commands.register_system(|data: Query<&mut TestData<f32>>| {
                for mut val in data {
                    val.0 += 1.0;
                }
            });
            world.flush();

            b.iter(|| {
                let world = app.world_mut();
                world.run_system(updater).unwrap();
                app.update();
                black_box(&mut app);
            });
        });
        group.bench_with_input(BenchmarkId::new("signal2", i), &i, |b, i| {
            let (mut app, updater) = v2::serial_chain(*i);
            b.iter(|| {
                app.world_mut().run_system(updater).unwrap();
                app.update();
                black_box(&mut app);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, observer_benches);
criterion_main!(benches);
