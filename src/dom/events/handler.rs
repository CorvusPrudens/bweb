use bevy_ecs::prelude::*;

use super::Ev;

pub struct Concise;
pub struct Fallible;

pub trait IntoHandlerSystem<E, M> {
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
