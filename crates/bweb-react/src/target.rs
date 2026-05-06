use bevy_app::prelude::*;
use bevy_ecs::{
    lifecycle::HookContext,
    prelude::*,
    query::{QueryData, QueryEntityError, QueryFilter, ROQueryItem},
    system::SystemParam,
    world::DeferredWorld,
};
use bevy_platform::collections::HashMap;

pub struct TargetPlugin;

impl Plugin for TargetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Targets>();
    }
}

#[derive(Component, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Debug)]
#[component(on_insert = Self::on_insert_hook, on_replace = Self::on_replace_hook)]
pub struct Target(uuid::Uuid);

impl Target {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    fn on_insert_hook(mut world: DeferredWorld, context: HookContext) {
        let value = *world
            .get::<Self>(context.entity)
            .expect("entity should have `Target` component");

        world
            .resource_mut::<Targets>()
            .0
            .insert(value, context.entity);
    }

    fn on_replace_hook(mut world: DeferredWorld, context: HookContext) {
        let value = *world
            .get::<Self>(context.entity)
            .expect("entity should have `Target` component");

        world.resource_mut::<Targets>().0.remove(&value);
    }
}

impl Default for Target {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Target({})", self.0)
    }
}

#[derive(Resource, Default)]
pub struct Targets(pub(crate) HashMap<Target, Entity>);

impl Targets {
    pub fn get(&self, target: &Target) -> Result<Entity> {
        self.0
            .get(target)
            .copied()
            .ok_or_else(|| format!("No entity found for target {}", target.0).into())
    }
}

#[derive(SystemParam)]
pub struct TQuery<'w, 's, D, F = ()>
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    targets: Res<'w, Targets>,
    query: Query<'w, 's, D, F>,
}

#[derive(Debug, Clone)]
pub enum TargetQueryError {
    NoSuchTarget(Target),
    Entity(QueryEntityError),
}

impl<'s, D, F> TQuery<'_, 's, D, F>
where
    D: QueryData,
    F: QueryFilter,
{
    #[inline]
    pub fn get(&self, target: Target) -> Result<ROQueryItem<'_, 's, D>, TargetQueryError> {
        let entity = *self
            .targets
            .0
            .get(&target)
            .ok_or(TargetQueryError::NoSuchTarget(target))?;
        self.query.get(entity).map_err(TargetQueryError::Entity)
    }

    #[inline]
    pub fn get_mut(&mut self, target: Target) -> Result<D::Item<'_, 's>, TargetQueryError> {
        let entity = *self
            .targets
            .0
            .get(&target)
            .ok_or(TargetQueryError::NoSuchTarget(target))?;
        self.query.get_mut(entity).map_err(TargetQueryError::Entity)
    }
}

impl core::error::Error for TargetQueryError {}

impl core::fmt::Display for TargetQueryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Entity(e) => e.fmt(f),
            Self::NoSuchTarget(t) => {
                write!(f, "The target {t} was not found")
            }
        }
    }
}

#[derive(Clone, Copy)]
pub enum EntityTarget {
    Entity(Entity),
    Target(Target),
}

impl EntityTarget {
    pub fn get(&self, targets: &Targets) -> Option<Entity> {
        match self {
            Self::Entity(e) => Some(*e),
            Self::Target(t) => targets.get(t).ok(),
        }
    }
}

impl From<Entity> for EntityTarget {
    fn from(value: Entity) -> Self {
        EntityTarget::Entity(value)
    }
}

impl From<Target> for EntityTarget {
    fn from(value: Target) -> Self {
        EntityTarget::Target(value)
    }
}
