use super::{DomSystems, html::Element};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::{
    component::ComponentId,
    lifecycle::HookContext,
    prelude::*,
    system::SystemChangeTick,
    world::{DeferredWorld, EntityRefExcept},
};
use bevy_platform::collections::HashMap;
use bevy_query_observer::{AddStopObserver, Stop};
use std::borrow::Cow;

pub(super) struct AttributePlugin;

impl Plugin for AttributePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Data::plugin);
        app.add_plugins(Download::plugin);
        app.add_systems(PostUpdate, update_attributes.in_set(DomSystems::Attach));
    }
}

macro_rules! attribute {
    ($ty:ident, $attr:literal) => {
        #[derive(Debug, Component, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[component(on_insert = insert_hook::<Self>, on_replace = Self::replace)]
        pub struct $ty(Cow<'static, str>);

        impl Attribute for $ty {
            fn set(&self, element: &Element) -> Result {
                element.set_attribute($attr, &self.0).js_err()
            }
        }

        impl core::ops::Deref for $ty {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl core::fmt::Write for $ty {
            fn write_str(&mut self, input: &str) -> core::fmt::Result {
                self.to_mut().push_str(input);

                Ok(())
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl $ty {
            pub fn new(attribute: impl Into<Cow<'static, str>>) -> Self {
                Self(attribute.into())
            }

            pub fn clear(&mut self) {
                match &mut self.0 {
                    Cow::Borrowed(_) => {
                        self.0 = Cow::Owned(String::new());
                    }
                    Cow::Owned(o) => {
                        o.clear();
                    }
                }
            }

            pub fn to_mut(&mut self) -> &mut String {
                self.0.to_mut()
            }

            fn replace(mut world: DeferredWorld, context: HookContext) {
                world
                    .commands()
                    .entity(context.entity)
                    .entry::<Attributes>()
                    .or_default()
                    .and_modify(move |mut attr| {
                        attr.remove(context.component_id, Cow::from($attr));
                    });
            }
        }
    };
}

trait Attribute {
    fn set(&self, element: &Element) -> Result;
}

#[derive(Component, Default)]
struct Attributes {
    attributes: Vec<(ComponentId, AttributeThunk)>,
    removed: HashMap<ComponentId, Cow<'static, str>>,
}

type AttributeThunk =
    for<'a> fn(&'a EntityRefExcept<(Attributes, Element)>) -> Option<&'a dyn Attribute>;

impl Attributes {
    pub fn insert<T: Component + Attribute>(&mut self, id: ComponentId) {
        self.insert_thunk(id, |entity: &EntityRefExcept<(Attributes, Element)>| {
            entity.get::<T>().map(|t| t as &dyn Attribute)
        });
    }

    fn insert_thunk(&mut self, id: ComponentId, thunk: AttributeThunk) {
        self.attributes.push((id, thunk));
        self.removed.remove(&id);
    }

    pub fn remove(&mut self, id: ComponentId, name: Cow<'static, str>) {
        self.attributes.retain(|a| a.0 != id);
        self.removed.insert(id, name);
    }
}

fn insert_hook<T: Component + Attribute>(mut world: DeferredWorld, context: HookContext) {
    world
        .commands()
        .entity(context.entity)
        .entry::<Attributes>()
        .or_default()
        .and_modify(move |mut attr| {
            attr.insert::<T>(context.component_id);
        });
}

fn update_attributes(
    mut attributes: Query<(
        &mut Attributes,
        &Element,
        EntityRefExcept<(Attributes, Element)>,
    )>,
    ticks: SystemChangeTick,
) -> Result {
    for (mut attributes, element, entity) in &mut attributes {
        for (_, attr) in attributes.removed.drain() {
            element.remove_attribute(&attr).js_err()?;
        }

        for (id, thunk) in &attributes.attributes {
            if entity
                .get_change_ticks_by_id(*id)
                .is_some_and(|t| t.is_changed(ticks.last_run(), ticks.this_run()))
                && let Some(attr) = thunk(&entity)
            {
                attr.set(element)?;
            }
        }
    }

    Ok(())
}

attribute! {Href, "href"}
attribute! {Title, "title"}
attribute! {Style, "style"}
attribute! {Width, "width"}
attribute! {Height, "height"}
attribute! {Src, "src"}
attribute! {Target, "target"}
attribute! {Tabindex, "tabindex"}
attribute! {D, "d"}
attribute! {Lang, "lang"}
attribute! {ViewBox, "viewBox"}
attribute! {Xmlns, "xmlns"}
attribute! {Fill, "fill"}
attribute! {Type, "type"}
attribute! {Accept, "accept"}
attribute! {AccessKey, "accesskey"}
attribute! {Action, "action"}
attribute! {Allow, "allow"}
attribute! {Alt, "alt"}
attribute! {As, "as"}
attribute! {Autocomplete, "autocomplete"}
attribute! {Cite, "cite"}
attribute! {Content, "content"}
attribute! {Coords, "coords"}
attribute! {Csp, "csp"}
attribute! {Datetime, "datetime"}
attribute! {Dirname, "dirname"}
attribute! {Enctype, "enctype"}
attribute! {ElementTiming, "elementtiming"}
attribute! {For, "for"}
attribute! {Form, "form"}
attribute! {FormAction, "formaction"}
attribute! {FormEnctype, "formenctype"}
attribute! {FormMethod, "formmethod"}
attribute! {FormNoValidate, "formnovalidate"}
attribute! {FormTarget, "formtarget"}
attribute! {Headers, "headers"}
attribute! {HrefLang, "hreflang"}
attribute! {Id, "id"}
attribute! {Integrity, "integrity"}
attribute! {ItemProp, "itemprop"}
attribute! {Label, "label"}
attribute! {List, "list"}
attribute! {Media, "media"}
attribute! {Name, "name"}
attribute! {Pattern, "pattern"}
attribute! {Ping, "ping"}
attribute! {Placeholder, "placeholder"}
attribute! {Poster, "poster"}
attribute! {Rel, "rel"}
attribute! {Role, "role"}
attribute! {Scope, "scope"}
attribute! {Shape, "shape"}
attribute! {Size, "size"}
attribute! {Sizes, "sizes"}
attribute! {Slot, "slot"}
attribute! {SrcDoc, "srcdoc"}
attribute! {SrcLang, "srclang"}
attribute! {SrcSet, "srcset"}
attribute! {Step, "step"}
attribute! {UseMap, "usemap"}
attribute! {Value, "value"}

macro_rules! boolean_attribute {
    ($ty:ident, $attr:literal) => {
        #[derive(Debug, Component, Clone, PartialEq, Eq)]
        #[component(on_insert = insert_hook::<Self>, on_replace = Self::replace)]
        pub struct $ty;

        impl Attribute for $ty {
            fn set(&self, element: &Element) -> Result {
                element.set_attribute($attr, "").js_err()
            }
        }

        impl $ty {
            fn replace(mut world: DeferredWorld, context: HookContext) {
                world
                    .commands()
                    .entity(context.entity)
                    .entry::<Attributes>()
                    .or_default()
                    .and_modify(move |mut attr| {
                        attr.remove(context.component_id, Cow::from($attr));
                    });
            }
        }
    };
}

boolean_attribute! {Muted, "muted"}
boolean_attribute! {Loop, "loop"}
boolean_attribute! {Disabled, "disabled"}
boolean_attribute! {Checked, "checked"}
boolean_attribute! {Alpha, "alpha"}
boolean_attribute! {Async, "async"}
boolean_attribute! {Autocapitalize, "autocapitalize"}
boolean_attribute! {Autoplay, "autoplay"}
boolean_attribute! {Controls, "controls"}
boolean_attribute! {Default, "default"}
boolean_attribute! {Defer, "defer"}
boolean_attribute! {IsMap, "ismap"}
boolean_attribute! {Multiple, "multiple"}
boolean_attribute! {NoValidate, "novalidate"}
boolean_attribute! {Open, "open"}
boolean_attribute! {PlaysInline, "playsinline"}
boolean_attribute! {ReadOnly, "readonly"}
boolean_attribute! {Required, "required"}
boolean_attribute! {Reversed, "reversed"}
boolean_attribute! {Sandbox, "sandbox"}
boolean_attribute! {Selected, "selected"}

macro_rules! enum_attribute {
    ($ty:ident, $attr:literal, $($var:ident, $value:literal),*) => {
        #[derive(Debug, Component, Clone, PartialEq, Eq)]
        #[component(on_insert = insert_hook::<Self>, on_replace = Self::replace)]
        pub enum $ty {
            $($var),*
        }

        impl Attribute for $ty {
            fn set(&self, element: &Element) -> Result {
                element.set_attribute($attr, self.as_attribute()).js_err()
            }
        }

        impl $ty {
            pub fn as_attribute(&self) -> &'static str {
                match self {
                    $(
                        Self::$var => $value,
                    )*
                }
            }

            fn replace(mut world: DeferredWorld, context: HookContext) {
                world
                    .commands()
                    .entity(context.entity)
                    .entry::<Attributes>()
                    .or_default()
                    .and_modify(move |mut attr| {
                        attr.remove(context.component_id, Cow::from($attr));
                    });
            }
        }
    };
}

enum_attribute!(
    Hidden,
    "hidden",
    Hidden,
    "hidden",
    UntilFound,
    "until-found"
);
enum_attribute!(Draggable, "draggable", True, "true", False, "false");
enum_attribute!(
    ContentEditable,
    "contenteditable",
    True,
    "true",
    False,
    "false",
    PlaintextOnly,
    "plaintext-only"
);
enum_attribute!(Capture, "capture", User, "user", Environment, "environment");
enum_attribute!(Charset, "charset", Utf8, "utf-8");
enum_attribute!(
    Colorspace,
    "colorspace",
    LimitedSrgb,
    "limited-srgb",
    DisplayP3,
    "display-p3"
);
enum_attribute!(
    Crossorigin,
    "crossorigin",
    Anonymous,
    "anonymous",
    UseCredentials,
    "use-credentials"
);
enum_attribute!(
    Decoding, "decoding", Sync, "sync", Async, "async", Auto, "auto"
);
enum_attribute!(Dir, "dir", Ltr, "ltr", Rtl, "rtl", Auto, "auto");
enum_attribute!(
    EnterKeyHint,
    "enterkeyhint",
    Enter,
    "enter",
    Done,
    "done",
    Go,
    "go",
    Next,
    "next",
    Previous,
    "previous",
    Search,
    "search",
    Send,
    "send"
);
enum_attribute!(
    FetchPriority,
    "fetchpriority",
    High,
    "high",
    Low,
    "low",
    Auto,
    "auto"
);
enum_attribute!(
    HttpEquiv,
    "http-equiv",
    ContentLanguage,
    "content-language",
    ContentType,
    "content-type",
    ContentSecurityPolicy,
    "content-security-policy",
    DefaultStyle,
    "default-style",
    Refresh,
    "refresh",
    SetCookie,
    "set-cookie"
);
enum_attribute!(
    InputMode,
    "inputmode",
    None,
    "none",
    Text,
    "text",
    Decimal,
    "decimal",
    Numeric,
    "numeric",
    Tel,
    "tel",
    Search,
    "search",
    Email,
    "email",
    Url,
    "url"
);
enum_attribute!(
    Kind,
    "kind",
    Subtitles,
    "subtitles",
    Captions,
    "captions",
    Descriptions,
    "descriptions",
    Chapters,
    "chapters",
    Metadata,
    "metadata"
);
enum_attribute!(Loading, "loading", Eager, "eager", Lazy, "lazy");
enum_attribute!(
    Method, "method", Post, "post", Get, "get", Dialog, "dialog", Submit, "submit"
);
enum_attribute!(
    Preload, "preload", None, "none", Metadata, "metadata", Auto, "auto"
);
enum_attribute!(
    ReferrerPolicy,
    "referrerpolicy",
    NoReferrer,
    "no-referrer",
    NoReferrerWhenDowngrade,
    "no-referrer-when-downgrade",
    Origin,
    "origin",
    OriginWhenCrossOrigin,
    "origin-when-cross-origin",
    SameOrigin,
    "same-origin",
    StrictOrigin,
    "strict-origin",
    StrictOriginWhenCrossOrigin,
    "strict-origin-when-cross-origin",
    UnsafeUrl,
    "unsafe-url"
);
enum_attribute!(Spellcheck, "spellcheck", True, "true", False, "false");
enum_attribute!(Translate, "translate", Yes, "yes", No, "no");
enum_attribute!(Wrap, "wrap", Hard, "hard", Soft, "soft", Off, "off");

macro_rules! value_attribute {
    ($ty:ident, $attr:literal, $inner:path) => {
        #[derive(Debug, Component, Clone, PartialEq, PartialOrd)]
        #[component(on_insert = insert_hook::<Self>, on_replace = Self::replace)]
        pub struct $ty(pub $inner);

        impl Attribute for $ty {
            fn set(&self, element: &Element) -> Result {
                let value = self.0.to_string();
                element.set_attribute($attr, &value).js_err()
            }
        }

        impl $ty {
            fn replace(mut world: DeferredWorld, context: HookContext) {
                world
                    .commands()
                    .entity(context.entity)
                    .entry::<Attributes>()
                    .or_default()
                    .and_modify(move |mut attr| {
                        attr.remove(context.component_id, Cow::from($attr));
                    });
            }
        }
    };
}

value_attribute!(Start, "start", u32);
value_attribute!(Cols, "cols", u32);
value_attribute!(Rows, "rows", u32);
value_attribute!(ColSpan, "colspan", u16);
value_attribute!(RowSpan, "rowspan", u16);
value_attribute!(Span, "span", u32);
value_attribute!(Min, "min", f32);
value_attribute!(Max, "max", f32);
value_attribute!(Low, "low", f32);
value_attribute!(High, "high", f32);
value_attribute!(Optimum, "optimum", f32);
value_attribute!(MaxLength, "maxlength", u32);
value_attribute!(MinLength, "maxlength", u32);

#[derive(Debug, Component, Clone, PartialEq, PartialOrd)]
pub enum Download {
    Auto,
    Filename(Cow<'static, str>),
}

impl Download {
    // TODO: these should really be trait-like
    fn attach(attrs: Query<(&Self, Option<&Element>), Changed<Self>>) -> Result {
        for (dl, element) in &attrs {
            let Some(element) = element else {
                return Err("'download' attribute requires an `Element`".into());
            };

            let value = match dl {
                Download::Filename(name) => name.clone(),
                Download::Auto => Cow::Borrowed(""),
            };

            element.set_attribute("download", &value).js_err()?;
        }

        Ok(())
    }

    fn observe_remove(trigger: On<Remove, Self>, attr: Query<&Element>) -> Result {
        let Ok(element) = attr.get(trigger.entity) else {
            return Ok(());
        };

        element.remove_attribute("download").js_err()
    }

    fn plugin(app: &mut App) {
        app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
            .add_observer(Self::observe_remove);
    }
}

#[derive(Debug, Component, Clone, PartialEq, PartialOrd)]
#[component(immutable)]
pub struct Data {
    name: Cow<'static, str>,
    value: Cow<'static, str>,
}

impl Data {
    pub fn new(kind: impl Into<Cow<'static, str>>, value: impl Into<Cow<'static, str>>) -> Self {
        Self {
            name: kind.into(),
            value: value.into(),
        }
    }

    fn attribute_string(&self) -> String {
        format!("data-{}", self.name)
    }

    // TODO: these should really be trait-like
    fn attach(attrs: Query<(&Self, Option<&Element>), Changed<Self>>) -> Result {
        for (data, element) in &attrs {
            let Some(element) = element else {
                return Err(format!("'data-{}' attribute requires an `Element`", data.name).into());
            };

            element
                .set_attribute(&data.attribute_string(), &data.value)
                .js_err()?;
        }

        Ok(())
    }

    fn remove(stop: Stop<(&Self, &Element)>) -> Result {
        let (data, element) = stop.into_inner();
        element.remove_attribute(&data.attribute_string()).js_err()
    }

    fn plugin(app: &mut App) {
        app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
            .add_stop_observer(Self::remove);
    }
}
