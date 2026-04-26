use super::HtmlElementName;
use bevy_ecs::prelude::*;

// Main Root
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("html"))]
pub struct Html;

// Document metadata
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("base"))]
pub struct Base;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("head"))]
pub struct Head;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("link"))]
pub struct Link;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("meta"))]
pub struct Meta;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("style"))]
pub struct StyleElement;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("title"))]
pub struct TitleElement;

// Sectioning root
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("body"))]
pub struct Body;

// Content sectioning
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("address"))]
pub struct Address;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("article"))]
pub struct Article;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("aside"))]
pub struct Aside;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("footer"))]
pub struct Footer;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("header"))]
pub struct Header;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("hgroup"))]
pub struct Hgroup;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("main"))]
pub struct Main;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("nav"))]
pub struct Nav;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("section"))]
pub struct Section;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("search"))]
pub struct Search;

// Text content
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("blockquote"))]
pub struct BlockQuote;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("dd"))]
pub struct Dd;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("div"))]
pub struct Div;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("dl"))]
pub struct Dl;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("dt"))]
pub struct Dt;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("figcaption"))]
pub struct FigCaption;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("figure"))]
pub struct Figure;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("hr"))]
pub struct Hr;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("li"))]
pub struct Li;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("menu"))]
pub struct Menu;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("ol"))]
pub struct Ol;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("p"))]
pub struct P;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("pre"))]
pub struct Pre;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("ul"))]
pub struct Ul;

// Inline text semantics
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("a"))]
pub struct A;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("abbr"))]
pub struct Abbr;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("b"))]
pub struct B;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("bdi"))]
pub struct Bdi;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("bdo"))]
pub struct Bdo;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("br"))]
pub struct Br;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("cite"))]
pub struct Cite;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("code"))]
pub struct Code;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("data"))]
pub struct Data;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("dfn"))]
pub struct Dfn;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("em"))]
pub struct Em;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("i"))]
pub struct I;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("kbd"))]
pub struct Kbd;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("mark"))]
pub struct Mark;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("q"))]
pub struct Q;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("rp"))]
pub struct Rp;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("rt"))]
pub struct Rt;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("ruby"))]
pub struct Ruby;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("s"))]
pub struct S;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("samp"))]
pub struct Samp;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("small"))]
pub struct Small;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("span"))]
pub struct Span;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("strong"))]
pub struct Strong;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("sub"))]
pub struct Sub;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("sup"))]
pub struct Sup;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("time"))]
pub struct Time;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("u"))]
pub struct U;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("var"))]
pub struct Var;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("wbr"))]
pub struct Wbr;

// Image and multimedia
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("area"))]
pub struct Area;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("audio"))]
pub struct Audio;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("img"))]
pub struct Img;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("map"))]
pub struct Map;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("track"))]
pub struct Track;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("video"))]
pub struct Video;

// Embedded content
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("embed"))]
pub struct Embed;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("fencedframe"))]
pub struct FencedFrame;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("iframe"))]
pub struct Iframe;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("object"))]
pub struct Object;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("picture"))]
pub struct Picture;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("source"))]
pub struct Source;

// Scripting
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("canvas"))]
pub struct Canvas;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("noscript"))]
pub struct NoScript;

// TODO: does this need special support?
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("script"))]
pub struct Script;

// Demarcating edits
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("del"))]
pub struct Del;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("ins"))]
pub struct Ins;

// Table content
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("caption"))]
pub struct Caption;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("col"))]
pub struct Col;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("colgroup"))]
pub struct ColGroup;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("table"))]
pub struct Table;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("tbody"))]
pub struct Tbody;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("td"))]
pub struct Td;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("tfoot"))]
pub struct Tfoot;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("th"))]
pub struct Th;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("thead"))]
pub struct Thead;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("tr"))]
pub struct Tr;

// Forms
#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("button"))]
pub struct Button;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("datalist"))]
pub struct DataList;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("fieldset"))]
pub struct FieldSet;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("form"))]
pub struct Form;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("input"))]
pub struct Input;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("label"))]
pub struct Label;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("legend"))]
pub struct Legend;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("meter"))]
pub struct Meter;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("optgroup"))]
pub struct OptGroup;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("option"))]
pub struct OptionElement;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("output"))]
pub struct Output;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("progress"))]
pub struct Progress;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("select"))]
pub struct Select;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("selectedcontent"))]
pub struct SelectedContent;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("textarea"))]
pub struct TextArea;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("h1"))]
pub struct H1;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("h2"))]
pub struct H2;

#[derive(Default, Component, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "debug", derive(Debug))]
#[require(HtmlElementName("h3"))]
pub struct H3;
