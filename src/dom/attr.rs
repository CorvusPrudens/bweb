use super::{DomSystems, html::Element};
use crate::js_err::JsErr;
use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
use std::borrow::Cow;

pub(super) struct AttributePlugin;

impl Plugin for AttributePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(Href::plugin);
        app.add_plugins(Height::plugin);
        app.add_plugins(D::plugin);
        app.add_plugins(Lang::plugin);
        app.add_plugins(ViewBox::plugin);
        app.add_plugins(Xmlns::plugin);
        app.add_plugins(Fill::plugin);

        app.add_plugins(Accept::plugin);
        app.add_plugins(AccessKey::plugin);
        app.add_plugins(Action::plugin);
        app.add_plugins(Allow::plugin);
        app.add_plugins(Alpha::plugin);
        app.add_plugins(Alt::plugin);
        app.add_plugins(Async::plugin);
        app.add_plugins(Autocapitalize::plugin);
        app.add_plugins(As::plugin);
        app.add_plugins(Autocomplete::plugin);
        app.add_plugins(Autoplay::plugin);
        app.add_plugins(Capture::plugin);
        app.add_plugins(Charset::plugin);
        app.add_plugins(Checked::plugin);
        app.add_plugins(Cite::plugin);
        app.add_plugins(Colorspace::plugin);
        app.add_plugins(Cols::plugin);
        app.add_plugins(ColSpan::plugin);
        app.add_plugins(Content::plugin);
        app.add_plugins(ContentEditable::plugin);
        app.add_plugins(Controls::plugin);
        app.add_plugins(Coords::plugin);
        app.add_plugins(Crossorigin::plugin);
        app.add_plugins(Csp::plugin);
        app.add_plugins(Data::plugin);
        app.add_plugins(Datetime::plugin);
        app.add_plugins(Decoding::plugin);
        app.add_plugins(Default::plugin);
        app.add_plugins(Defer::plugin);
        app.add_plugins(Dir::plugin);
        app.add_plugins(Dirname::plugin);
        app.add_plugins(Disabled::plugin);
        app.add_plugins(Download::plugin);
        app.add_plugins(Draggable::plugin);
        app.add_plugins(Enctype::plugin);
        app.add_plugins(EnterKeyHint::plugin);
        app.add_plugins(ElementTiming::plugin);
        app.add_plugins(FetchPriority::plugin);
        app.add_plugins(For::plugin);
        app.add_plugins(Form::plugin);
        app.add_plugins(FormAction::plugin);
        app.add_plugins(FormEnctype::plugin);
        app.add_plugins(FormMethod::plugin);
        app.add_plugins(FormNoValidate::plugin);
        app.add_plugins(FormTarget::plugin);
        app.add_plugins(Headers::plugin);
        app.add_plugins(Hidden::plugin);
        app.add_plugins(High::plugin);
        app.add_plugins(HrefLang::plugin);
        app.add_plugins(HttpEquiv::plugin);
        app.add_plugins(Id::plugin);
        app.add_plugins(Integrity::plugin);
        app.add_plugins(InputMode::plugin);
        app.add_plugins(IsMap::plugin);
        app.add_plugins(ItemProp::plugin);
        app.add_plugins(Kind::plugin);
        app.add_plugins(Label::plugin);
        app.add_plugins(Loading::plugin);
        app.add_plugins(List::plugin);
        app.add_plugins(Loop::plugin);
        app.add_plugins(Low::plugin);
        app.add_plugins(Max::plugin);
        app.add_plugins(MaxLength::plugin);
        app.add_plugins(MinLength::plugin);
        app.add_plugins(Media::plugin);
        app.add_plugins(Method::plugin);
        app.add_plugins(Min::plugin);
        app.add_plugins(Multiple::plugin);
        app.add_plugins(Muted::plugin);
        app.add_plugins(Name::plugin);
        app.add_plugins(NoValidate::plugin);
        app.add_plugins(Open::plugin);
        app.add_plugins(Optimum::plugin);
        app.add_plugins(Pattern::plugin);
        app.add_plugins(Ping::plugin);
        app.add_plugins(Placeholder::plugin);
        app.add_plugins(PlaysInline::plugin);
        app.add_plugins(Poster::plugin);
        app.add_plugins(Preload::plugin);
        app.add_plugins(ReadOnly::plugin);
        app.add_plugins(ReferrerPolicy::plugin);
        app.add_plugins(Rel::plugin);
        app.add_plugins(Required::plugin);
        app.add_plugins(Reversed::plugin);
        app.add_plugins(Role::plugin);
        app.add_plugins(Rows::plugin);
        app.add_plugins(RowSpan::plugin);
        app.add_plugins(Sandbox::plugin);
        app.add_plugins(Scope::plugin);
        app.add_plugins(Selected::plugin);
        app.add_plugins(Shape::plugin);
        app.add_plugins(Size::plugin);
        app.add_plugins(Sizes::plugin);
        app.add_plugins(Slot::plugin);
        app.add_plugins(Span::plugin);
        app.add_plugins(Spellcheck::plugin);
        app.add_plugins(Src::plugin);
        app.add_plugins(SrcDoc::plugin);
        app.add_plugins(SrcLang::plugin);
        app.add_plugins(SrcSet::plugin);
        app.add_plugins(Start::plugin);
        app.add_plugins(Step::plugin);
        app.add_plugins(Style::plugin);
        app.add_plugins(Tabindex::plugin);
        app.add_plugins(Target::plugin);
        app.add_plugins(Title::plugin);
        app.add_plugins(Translate::plugin);
        app.add_plugins(Type::plugin);
        app.add_plugins(UseMap::plugin);
        app.add_plugins(Value::plugin);
        app.add_plugins(Width::plugin);
        app.add_plugins(Wrap::plugin);
    }
}

macro_rules! attribute {
    ($ty:ident, $attr:literal) => {
        #[derive(Debug, Component, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $ty(Cow<'static, str>);

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

            // TODO: these should really be trait-like
            fn attach(attrs: Query<(&Self, Option<&Element>), Changed<Self>>) -> Result {
                for (href, element) in &attrs {
                    let Some(element) = element else {
                        return Err(format!("'{}' attribute requires an `Element`", $attr).into());
                    };

                    element.set_attribute($attr, &href.0).js_err()?;
                }

                Ok(())
            }

            fn observe_remove(trigger: On<Remove, Self>, attr: Query<&Element>) -> Result {
                let Ok(element) = attr.get(trigger.entity) else {
                    return Ok(());
                };

                element.remove_attribute($attr).js_err()
            }

            fn plugin(app: &mut App) {
                app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
                    .add_observer(Self::observe_remove);
            }
        }
    };
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
attribute! {Data, "data"}
// TODO: add bespoke data-*
attribute! {Datetime, "datetime"}
attribute! {Dirname, "dirname"}
// TODO: add bespoke download
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
        pub struct $ty;

        impl $ty {
            // TODO: these should really be trait-like
            fn attach(attrs: Query<Option<&Element>, (Changed<Self>, With<Self>)>) -> Result {
                for element in &attrs {
                    let Some(element) = element else {
                        return Err(format!("'{}' attribute requires a DOM Element", $attr).into());
                    };

                    element.set_attribute($attr, "").js_err()?;
                }

                Ok(())
            }

            fn observe_remove(trigger: On<Remove, Self>, attr: Query<&Element>) -> Result {
                let Ok(element) = attr.get(trigger.entity) else {
                    return Ok(());
                };

                element.remove_attribute($attr).js_err()
            }

            fn plugin(app: &mut App) {
                app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
                    .add_observer(Self::observe_remove);
            }
        }
    };
}

boolean_attribute! {Muted, "muted"}
boolean_attribute! {Loop, "loop"}
boolean_attribute! {Disabled, "disabled"}
boolean_attribute! {Download, "download"}
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
        pub enum $ty {
            $($var),*
        }

        impl $ty {
            pub fn as_attribute(&self) -> &'static str {
                match self {
                    $(
                        Self::$var => $value,
                    )*
                }
            }

            // TODO: these should really be trait-like
            fn attach(attrs: Query<(&Self, Option<&Element>), Changed<Self>>) -> Result {
                for (attr, element) in &attrs {
                    let Some(element) = element else {
                        return Err(format!("'{}' attribute requires a DOM Element", $attr).into());
                    };

                    element.set_attribute($attr, attr.as_attribute()).js_err()?;
                }

                Ok(())
            }

            fn observe_remove(trigger: On<Remove, Self>, attr: Query<&Element>) -> Result {
                let Ok(element) = attr.get(trigger.entity) else {
                    return Ok(());
                };

                element.remove_attribute($attr).js_err()
            }

            pub(crate) fn plugin(app: &mut App) {
                app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
                    .add_observer(Self::observe_remove);
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
        pub struct $ty(pub $inner);

        impl $ty {
            // TODO: these should really be trait-like
            fn attach(attrs: Query<(&Self, Option<&Element>), Changed<Self>>) -> Result {
                for (href, element) in &attrs {
                    let Some(element) = element else {
                        return Err(format!("'{}' attribute requires an `Element`", $attr).into());
                    };

                    let value = href.0.to_string();
                    element.set_attribute($attr, &value).js_err()?;
                }

                Ok(())
            }

            fn observe_remove(trigger: On<Remove, Self>, attr: Query<&Element>) -> Result {
                let Ok(element) = attr.get(trigger.entity) else {
                    return Ok(());
                };

                element.remove_attribute($attr).js_err()
            }

            fn plugin(app: &mut App) {
                app.add_systems(PostUpdate, (Self::attach.in_set(DomSystems::Attach),))
                    .add_observer(Self::observe_remove);
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
