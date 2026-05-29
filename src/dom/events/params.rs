use crate::dom::events::EventOf;
use crate::dom::html::HtmlInputElement;
use bevy_ecs::prelude::*;
use bevy_ecs::system::{Query, SystemParam};

#[derive(SystemParam, Debug)]
pub struct InputValue<'w, 's> {
    ev: Query<'w, 's, &'static EventOf>,
    q: Query<'w, 's, &'static HtmlInputElement>,
}

impl InputValue<'_, '_> {
    pub fn get(&self, entity: Entity) -> Result<String> {
        let el = self.ev.get(entity)?;
        Ok(self.q.get(el.0)?.value())
    }
}
