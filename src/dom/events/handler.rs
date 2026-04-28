use bevy_ecs::prelude::*;

use super::Ev;

/// A marker type for concise handlers.
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct Concise;

/// A marker type for fallible handlers.
#[cfg_attr(feature = "debug", derive(Debug))]
pub struct Fallible;

/// Conversion trait to turn a function or closure into an
/// event-handling system.
pub trait IntoHandlerSystem<E, M> {
    /// Convert `Self` into an event-handling system.
    fn into_handler(self) -> impl System<In = Ev<E>, Out = Result>;
}

impl<S, E, M> IntoHandlerSystem<E, (M,)> for S
where
    S: IntoSystem<Ev<E>, (), M>,
    E: 'static,
{
    fn into_handler(self) -> impl System<In = Ev<E>, Out = Result> {
        IntoSystem::into_system(self.map(|_| Ok(())))
    }
}

impl<S, E, M> IntoHandlerSystem<E, (M, Concise)> for S
where
    S: IntoSystem<(), (), M>,
    E: 'static,
{
    fn into_handler(self) -> impl System<In = Ev<E>, Out = Result> {
        let input = |_: Ev<E>| ();

        IntoSystem::into_system(input.pipe(self).map(|_| Ok(())))
    }
}

// impl<S, E, M> IntoHandlerSystem<E, (M, Fallible)> for S
// where
//     S: IntoSystem<Ev<E>, Result, M>,
//     E: 'static,
// {
//     fn into_handler(self) -> impl System<In = Ev<E>, Out = Result> {
//         IntoSystem::into_system(self)
//     }
// }
//
// impl<S, E, M> IntoHandlerSystem<E, (M, Fallible, Concise)> for S
// where
//     S: IntoSystem<(), Result, M>,
//     E: 'static,
// {
//     fn into_handler(self) -> impl System<In = Ev<E>, Out = Result> {
//         let input = |_: Ev<E>| ();
//
//         IntoSystem::into_system(input.pipe(self))
//     }
// }
