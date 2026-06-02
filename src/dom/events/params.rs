use crate::dom::events::EventOf;
use crate::dom::html::{HtmlInputElement, HtmlTextAreaElement};
use bevy_ecs::prelude::*;
use bevy_ecs::system::{Query, SystemParam};

#[derive(SystemParam, Debug)]
pub struct InputValue<'w, 's> {
    ev: Query<'w, 's, &'static EventOf>,
    q: Query<'w, 's, AnyOf<(&'static HtmlInputElement, &'static HtmlTextAreaElement)>>,
}

impl InputValue<'_, '_> {
    pub fn get(&self, entity: Entity) -> Result<String> {
        let el = self.ev.get(entity)?;

        match self.q.get(el.0)? {
            (Some(input), _) => Ok(input.value()),
            (_, Some(text_area)) => Ok(text_area.value()),
            _ => unreachable!(),
        }
    }
}
