use bevy_ecs::{lifecycle::HookContext, prelude::*, world::DeferredWorld};

pub trait IntoOptionalBundle {
    type InnerValue: Bundle;
    fn into_optional(self) -> OptionalBundle<Self::InnerValue>;
}

impl<C: Bundle> IntoOptionalBundle for Option<C> {
    type InnerValue = C;

    fn into_optional(self) -> OptionalBundle<Self::InnerValue> {
        OptionalBundle(self)
    }
}

#[derive(Component)]
#[component(on_insert = Self::insert)]
pub struct OptionalBundle<C: Bundle>(pub Option<C>);

impl<C: Bundle> OptionalBundle<C> {
    pub const fn none() -> Self {
        Self(None)
    }
}

impl<C: Bundle> OptionalBundle<C> {
    fn insert(mut world: DeferredWorld, context: HookContext) {
        world.commands().queue(move |world: &mut World| -> Result {
            let mut component = world.get_entity_mut(context.entity)?;
            if let Some(inner_value) = component.take::<Self>().and_then(|mut o| o.0.take()) {
                component.insert(inner_value);
            }

            Ok(())
        });
    }
}
