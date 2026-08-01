use std::any::TypeId;

use bevy_app::{Plugin, PostUpdate};
use bevy_ecs::{
    lifecycle::HookContext,
    prelude::*,
    query::{QueryData, QueryFilter, ReadOnlyQueryData},
    system::BoxedReadOnlySystem,
    world::DeferredWorld,
};
use bevy_platform::collections::HashSet;
use bevy_utils::TypeIdMap;

pub mod mapped;
pub mod source;

use smallvec::SmallVec;
use source::SourceSignal;

use crate::signal2::source::SourceSignalView;

pub struct ReactivePlugin;

impl Plugin for ReactivePlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.init_resource::<ReactiveSystems>()
            .init_resource::<InnerReactiveSystems>()
            .add_systems(PostUpdate, run_reactive_schedule);
    }
}

pub trait SignalData: ReadOnlyQueryData + 'static {
    type Filter: QueryFilter;
}

impl<T: Component> SignalData for &'static T {
    type Filter = Changed<T>;
}

macro_rules! impl_signal_data {
    ($($ty:ident),*) => {
        impl<$($ty: SignalData),*> SignalData for ($($ty,)*) {
            type Filter = Or<($($ty::Filter,)*)>;
        }
    };
}

impl_signal_data!(A);
impl_signal_data!(A, B);
impl_signal_data!(A, B, C);
impl_signal_data!(A, B, C, D);

pub trait IntoSignalSystem {
    fn into_signal_system() -> BoxedReadOnlySystem;
}

impl<D: SignalData> IntoSignalSystem for D
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    fn into_signal_system() -> BoxedReadOnlySystem {
        let system = |query: Query<(D, &SubscriberSet<D>), D::Filter>, mut commands: Commands| {
            for (data, subs) in query {
                for closure in &subs.closures {
                    closure(data, commands.reborrow());
                }
            }
        };

        Box::new(IntoSystem::into_system(system))
    }
}

// impl<D: QueryData> Subscribers<D> {
//     pub fn insert_effect(
//         &mut self,
//         entity: Entity,
//         f: impl for<'w, 's> Fn(D::Item<'w, 's>, Commands) + Send + Sync + 'static,
//     ) {
//         self.entities
//             .entry(entity)
//             .or_default()
//             .closures
//             .push(Box::new(f))
//     }
// }

#[derive(Component)]
#[component(on_add = Self::add)]
struct SubscriberSet<D: SignalData>
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    closures: SmallVec<[SubscriberClosure<D>; 1]>,
}

impl<D: SignalData> SubscriberSet<D>
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    fn add(mut world: DeferredWorld, _: HookContext) {
        world.resource_mut::<ReactiveSystems>().register::<D>();
    }
}

impl<D: SignalData> Default for SubscriberSet<D>
where
    for<'w, 's> D::Item<'w, 's>: Copy,
{
    fn default() -> Self {
        Self {
            closures: SmallVec::new(),
        }
    }
}

type SubscriberClosure<D> = Box<
    dyn for<'w, 's, 'ww, 'ss> Fn(<D as QueryData>::Item<'w, 's>, Commands<'ww, 'ss>) + Send + Sync,
>;

pub trait SignalExt<'w, 's> {
    fn signal<D>(&mut self) -> SourceSignalView<'_, 'w, 's, D>
    where
        D: SignalData,
        for<'ww, 'ss> D::Item<'ww, 'ss>: Copy;
}

impl<'w, 's> SignalExt<'w, 's> for Commands<'w, 's> {
    fn signal<D>(&mut self) -> SourceSignalView<'_, 'w, 's, D>
    where
        D: SignalData,
        for<'ww, 'ss> D::Item<'ww, 'ss>: Copy,
    {
        let signal = SourceSignal::from_commands(self);
        SourceSignalView(signal, self)
    }
}

#[derive(Resource, Default)]
pub struct ReactiveSystems {
    /// The set of all systems ever registered.
    registered: HashSet<TypeId>,
    /// Systems that haven't been drained.
    systems: Vec<(TypeId, BoxedReadOnlySystem)>,
}

impl ReactiveSystems {
    /// Idempotently register a basic signal system.
    pub fn register<D>(&mut self) -> bool
    where
        D: SignalData,
        for<'w, 's> D::Item<'w, 's>: Copy,
    {
        let id = TypeId::of::<D>();
        if self.registered.insert(id) {
            self.systems
                .push((id, <D as IntoSignalSystem>::into_signal_system()));
            true
        } else {
            false
        }
    }
}

/// Systems that have been drained from the `ReactiveSystems` resource.
#[derive(Resource, Default)]
struct InnerReactiveSystems {
    source_systems: TypeIdMap<BoxedReadOnlySystem>,
}

pub fn run_reactive_schedule(world: &mut World) {
    world.resource_scope(|world, mut outer: Mut<ReactiveSystems>| {
        for (_, system) in outer.systems.iter_mut() {
            system.initialize(world);
        }

        let mut inner = world.resource_mut::<InnerReactiveSystems>();
        inner.source_systems.extend(outer.systems.drain(..));
    });

    let errors = world.resource_scope(|world, mut inner: Mut<InnerReactiveSystems>| {
        let mut errors = Vec::new();
        for system in inner.source_systems.values_mut() {
            if let Err(e) = system.run((), world) {
                errors.push(e);
            }
        }

        errors
    });

    if !errors.is_empty() {
        // TOOD: just forward these to the error handler
        log::error!("Failed to evaluate all reactive systems: {errors:#?}");
    }
}

fn test(mut cmd: Commands, model: Entity) {
    #[derive(Component)]
    struct Test;

    let name = cmd
        .signal::<&Name>()
        .watch(model)
        .map(|name| Name::new(format!("wow! {name}")));

    let name_and_test = cmd.signal::<(&Name, &Test)>().watch(model);
}
