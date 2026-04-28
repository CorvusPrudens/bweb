use bell_react::prelude::*;
use bell_react::signal::signal_world::SWorld;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use criterion::{
    AxisScale, BenchmarkId, Criterion, PlotConfiguration, criterion_group, criterion_main,
};
use std::hint::black_box;

fn observer_benches(c: &mut Criterion) {
    #[derive(Component, PartialEq, Clone)]
    struct TestData<T>(pub T);

    let mut group = c.benchmark_group("run with update");
    for i in (0..5).map(|i| 10usize.pow(i)) {
        // group.bench_with_input(BenchmarkId::new("old", i), &i, |b, i| {
        //     use bell_react::*;

        //     let mut app = App::new();
        //     app.add_plugins(ReactPlugin);
        //     let mut world = app.world_mut();

        //     let test_entity = world.spawn(TestData(1f32)).id();

        //     for _ in 0..*i {
        //         let mut commands = world.commands();
        //         let derived = commands.derive(move |data: Query<&TestData<f32>>| {
        //             data.get(test_entity).unwrap().clone()
        //         });
        //         commands.spawn(derived);
        //     }

        //     b.iter(|| {
        //         app.world_mut()
        //             .get_mut::<TestData<f32>>(test_entity)
        //             .unwrap()
        //             .0 += 0.1;
        //         app.update();
        //         black_box(&mut app);
        //     });
        // });

        group.bench_with_input(BenchmarkId::new("new", i), &i, |b, i| {
            let mut app = App::new();
            app.add_plugins(ReactPlugin);
            let world = app.world_mut();

            let test_entity = world.spawn(TestData(1f32)).id();

            for _ in 0..*i {
                let mut commands = world.commands();
                let derived = commands.derive(move |data: SQuery<&TestData<f32>>| {
                    data.get(test_entity).unwrap().clone()
                });
                commands.spawn(derived);
            }

            b.iter(|| {
                app.world_mut()
                    .get_mut::<TestData<f32>>(test_entity)
                    .unwrap()
                    .0 += 0.1;
                app.update();
                black_box(&mut app);
            });
        });
    }
    group.finish();

    let mut group = c.benchmark_group("run without update");
    for i in (0..5).map(|i| 10usize.pow(i)) {
        //     group.bench_with_input(BenchmarkId::new("old", i), &i, |b, i| {
        //         use bell_react::*;

        //         let mut app = App::new();
        //         app.add_plugins(ReactPlugin);
        //         let mut world = app.world_mut();

        //         let test_entity = world.spawn(TestData(1f32)).id();

        //         for _ in 0..*i {
        //             let mut commands = world.commands();
        //             let derived = commands.derive(move |data: Query<&TestData<f32>>| {
        //                 data.get(test_entity).unwrap().clone()
        //             });
        //             commands.spawn(derived);
        //         }
        //         app.world_mut()
        //             .get_mut::<TestData<f32>>(test_entity)
        //             .unwrap()
        //             .0 += 0.1;
        //         app.update();

        //         b.iter(|| {
        //             app.update();
        //             black_box(&mut app);
        //         });
        //     });

        group.bench_with_input(BenchmarkId::new("new", i), &i, |b, i| {
            let mut app = App::new();
            app.add_plugins(ReactPlugin);
            let world = app.world_mut();

            let test_entity = world.spawn(TestData(1f32)).id();

            for _ in 0..*i {
                let mut commands = world.commands();
                let derived = commands.derive(move |data: SQuery<&TestData<f32>>| {
                    data.get(test_entity).unwrap().clone()
                });
                commands.spawn(derived);
            }

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
    }
    group.finish();

    let mut group = c.benchmark_group("multiple sources");
    for i in (0..5).map(|i| 10usize.pow(i)) {
        // group.bench_with_input(BenchmarkId::new("old", i), &i, |b, i| {
        //     use bell_react::*;

        //     let mut app = App::new();
        //     app.add_plugins(ReactPlugin);
        //     let mut world = app.world_mut();

        //     fn spawn_signals<T: PartialEq + Clone + Send + Sync + 'static>(
        //         value: T,
        //         total: usize,
        //         world: &mut World,
        //     ) -> Entity {
        //         let entity = world.spawn(TestData(value)).id();
        //         for _ in 0..total {
        //             let mut commands = world.commands();
        //             let derived = commands
        //                 .derive(move |data: Query<&TestData<T>>| data.get(entity).unwrap().clone());
        //             commands.spawn(derived);
        //         }
        //         entity
        //     }

        //     let float = spawn_signals(1f32, *i, world);
        //     let int = spawn_signals(1i32, *i, world);
        //     let string = spawn_signals(uuid::Uuid::new_v4().simple().to_string(), *i, world);
        //     let bool = spawn_signals(false, *i, world);

        //     b.iter(|| {
        //         app.world_mut().get_mut::<TestData<f32>>(float).unwrap().0 += 0.1;
        //         app.world_mut().get_mut::<TestData<i32>>(int).unwrap().0 += 1;
        //         app.world_mut()
        //             .get_mut::<TestData<String>>(string)
        //             .unwrap()
        //             .0 = uuid::Uuid::new_v4().simple().to_string();

        //         let val = app.world_mut().get_mut::<TestData<bool>>(bool).unwrap().0;
        //         app.world_mut().get_mut::<TestData<bool>>(bool).unwrap().0 = !val;

        //         app.update();
        //         black_box(&mut app);
        //     });
        // });

        group.bench_with_input(BenchmarkId::new("new", i), &i, |b, i| {
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
    }
    group.finish();

    let mut group = c.benchmark_group("multiple sources without update");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));
    for i in (0..5).map(|i| 10usize.pow(i)) {
        // group.bench_with_input(BenchmarkId::new("old", i), &i, |b, i| {
        //     use bell_react::*;

        //     let mut app = App::new();
        //     app.add_plugins(ReactPlugin);
        //     let mut world = app.world_mut();

        //     fn spawn_signals<T: PartialEq + Clone + Send + Sync + 'static>(
        //         value: T,
        //         total: usize,
        //         world: &mut World,
        //     ) -> Entity {
        //         let entity = world.spawn(TestData(value)).id();
        //         for _ in 0..total {
        //             let mut commands = world.commands();
        //             let derived = commands
        //                 .derive(move |data: Query<&TestData<T>>| data.get(entity).unwrap().clone());
        //             commands.spawn(derived);
        //         }
        //         entity
        //     }

        //     let float = spawn_signals(1f32, *i, world);
        //     let int = spawn_signals(1i32, *i, world);
        //     let string = spawn_signals(uuid::Uuid::new_v4().simple().to_string(), *i, world);
        //     let bool = spawn_signals(false, *i, world);

        //     app.world_mut().get_mut::<TestData<f32>>(float).unwrap().0 += 0.1;
        //     app.world_mut().get_mut::<TestData<i32>>(int).unwrap().0 += 1;
        //     app.world_mut()
        //         .get_mut::<TestData<String>>(string)
        //         .unwrap()
        //         .0 = uuid::Uuid::new_v4().simple().to_string();

        //     let val = app.world_mut().get_mut::<TestData<bool>>(bool).unwrap().0;
        //     app.world_mut().get_mut::<TestData<bool>>(bool).unwrap().0 = !val;
        //     app.update();

        //     b.iter(|| {
        //         app.update();
        //         black_box(&mut app);
        //     });
        // });

        group.bench_with_input(BenchmarkId::new("new", i), &i, |b, i| {
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
    }
    group.finish();

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
            // app.world_mut()
            //     .get_mut::<TestData<f32>>(test_entity)
            //     .unwrap()
            //     .0 += 0.1;
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
            // app.world_mut()
            //     .get_mut::<TestData<f32>>(test_entity)
            //     .unwrap()
            //     .0 += 0.1;
            app.update();
            black_box(&mut app);
        });
    });
    group.finish();

    let mut group = c.benchmark_group("spawn speed");
    group.bench_with_input("world", &1000, |b, i| {
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
    group.bench_with_input("query", &1000, |b, i| {
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
    group.finish();
}

criterion_group!(benches, observer_benches);
criterion_main!(benches);
