use bevy_ecs::{
    prelude::*,
    system::{IntoResult, RunSystemError, SystemParam, SystemParamItem, SystemState},
};

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid system",
    label = "invalid system"
)]
pub trait SystemOnceFunction<Marker> {
    /// The input type of this system. See [`System::In`].
    type In: SystemInput;
    /// The return type of this system. See [`System::Out`].
    type Out;

    /// The [`SystemParam`]/s used by this system to access the [`World`].
    type Param: SystemParam;

    /// Executes this system once. See [`System::run`] or [`System::run_unsafe`].
    fn run(
        self,
        input: <Self::In as SystemInput>::Inner<'_>,
        param_value: SystemParamItem<Self::Param>,
    ) -> Self::Out;
}

/// A marker type used to distinguish function systems with and without input.
#[doc(hidden)]
pub struct HasSystemInput;

use core::marker::PhantomData;

macro_rules! impl_system_function {
    ($($param: ident),*) => {
        #[expect(
            clippy::allow_attributes,
            reason = "This is within a macro, and as such, the below lints may not always apply."
        )]
        #[allow(
            non_snake_case,
            reason = "Certain variable names are provided by the caller, not by us."
        )]
        impl<Out, Func, $($param: SystemParam),*> SystemOnceFunction<fn($($param,)*) -> Out> for Func
        where
            Func:
                FnOnce($($param),*) -> Out +
                FnOnce($(SystemParamItem<$param>),*) -> Out,
            Out: 'static
        {
            type In = ();
            type Out = Out;
            type Param = ($($param,)*);

            #[inline]
            #[allow(clippy::too_many_arguments)]
            fn run(self, _input: (), param_value: SystemParamItem< ($($param,)*)>) -> Out {
                // Yes, this is strange, but `rustc` fails to compile this impl
                // without using this function. It fails to recognize that `func`
                // is a function, potentially because of the multiple impls of `FnOnce`
                fn call_inner<Out, $($param,)*>(
                    f: impl FnOnce($($param,)*)->Out,
                    $($param: $param,)*
                )->Out{
                    f($($param,)*)
                }
                let ($($param,)*) = param_value;
                call_inner(self, $($param),*)
            }
        }

        #[expect(
            clippy::allow_attributes,
            reason = "This is within a macro, and as such, the below lints may not always apply."
        )]
        #[allow(
            non_snake_case,
            reason = "Certain variable names are provided by the caller, not by us."
        )]
        impl<In, Out, Func, $($param: SystemParam),*> SystemOnceFunction<(HasSystemInput, fn(In, $($param,)*) -> Out)> for Func
        where
            Func:
                FnOnce(In, $($param),*) -> Out +
                FnOnce(In::Param<'_>, $(SystemParamItem<$param>),*) -> Out,
            In: SystemInput + 'static,
            Out: 'static
        {
            type In = In;
            type Out = Out;
            type Param = ($($param,)*);

            #[inline]
            #[allow(clippy::too_many_arguments)]
            fn run(self, input: In::Inner<'_>, param_value: SystemParamItem< ($($param,)*)>) -> Out {
                fn call_inner<In: SystemInput, Out, $($param,)*>(
                    _: PhantomData<In>,
                    f: impl FnOnce(In::Param<'_>, $($param,)*)->Out,
                    input: In::Inner<'_>,
                    $($param: $param,)*
                )->Out{
                    f(In::wrap(input), $($param,)*)
                }
                let ($($param,)*) = param_value;
                call_inner(PhantomData::<In>, self, input, $($param),*)
            }
        }
    };
}

impl_system_function!();
impl_system_function!(F0);
impl_system_function!(F0, F1);
impl_system_function!(F0, F1, F2);
impl_system_function!(F0, F1, F2, F3);
impl_system_function!(F0, F1, F2, F3, F4);
impl_system_function!(F0, F1, F2, F3, F4, F5);
impl_system_function!(F0, F1, F2, F3, F4, F5, F6);

pub trait RunSystemOnceOnce: Sized {
    fn run_once<T, Out, Marker>(self, system: T) -> Result<Out, RunSystemError>
    where
        T: SystemOnceFunction<Marker, In = (), Out: IntoResult<Out>>,
        T::Param: 'static,
    {
        self.run_once_with(system, ())
    }

    fn run_once_with<T, In, Out, Marker>(
        self,
        system: T,
        input: <In as SystemInput>::Inner<'_>,
    ) -> Result<Out, RunSystemError>
    where
        T: SystemOnceFunction<Marker, In = In, Out: IntoResult<Out>>,
        T::Param: 'static,
        In: SystemInput;
}

impl RunSystemOnceOnce for &mut World {
    fn run_once_with<T, In, Out, Marker>(
        self,
        system: T,
        input: <In as SystemInput>::Inner<'_>,
    ) -> Result<Out, RunSystemError>
    where
        T: SystemOnceFunction<Marker, In = In, Out: IntoResult<Out>>,
        T::Param: 'static,
        In: SystemInput,
    {
        let mut state = SystemState::<T::Param>::new(self);
        let param = state.get_mut(self);
        let out = system.run(input, param);
        state.apply(self);
        IntoResult::<Out>::into_result(out)
    }
}
