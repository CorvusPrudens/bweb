use super::HtmlElementName;
use bevy_ecs::prelude::*;

// Main Root
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("html"))]
pub struct Html;

// Document metadata
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("base"))]
pub struct Base;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("head"))]
pub struct Head;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("link"))]
pub struct Link;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("meta"))]
pub struct Meta;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("style"))]
pub struct StyleElement;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("title"))]
pub struct TitleElement;

// Sectioning root
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("body"))]
pub struct Body;

// Content sectioning
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("address"))]
pub struct Address;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("article"))]
pub struct Article;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("aside"))]
pub struct Aside;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("footer"))]
pub struct Footer;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("header"))]
pub struct Header;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("hgroup"))]
pub struct Hgroup;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("main"))]
pub struct Main;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("nav"))]
pub struct Nav;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("section"))]
pub struct Section;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("search"))]
pub struct Search;

// Text content
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("blockquote"))]
pub struct BlockQuote;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("dd"))]
pub struct Dd;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("div"))]
pub struct Div;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("dl"))]
pub struct Dl;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("dt"))]
pub struct Dt;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("figcaption"))]
pub struct FigCaption;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("figure"))]
pub struct Figure;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("hr"))]
pub struct Hr;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("li"))]
pub struct Li;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("menu"))]
pub struct Menu;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("ol"))]
pub struct Ol;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("p"))]
pub struct P;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("pre"))]
pub struct Pre;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("ul"))]
pub struct Ul;

// Inline text semantics
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("a"))]
pub struct A;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("abbr"))]
pub struct Abbr;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("b"))]
pub struct B;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("bdi"))]
pub struct Bdi;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("bdo"))]
pub struct Bdo;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("br"))]
pub struct Br;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("cite"))]
pub struct Cite;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("code"))]
pub struct Code;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("data"))]
pub struct Data;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("dfn"))]
pub struct Dfn;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("em"))]
pub struct Em;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("i"))]
pub struct I;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("kbd"))]
pub struct Kbd;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("mark"))]
pub struct Mark;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("q"))]
pub struct Q;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("rp"))]
pub struct Rp;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("rt"))]
pub struct Rt;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("ruby"))]
pub struct Ruby;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("s"))]
pub struct S;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("samp"))]
pub struct Samp;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("small"))]
pub struct Small;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("span"))]
pub struct Span;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("strong"))]
pub struct Strong;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("sub"))]
pub struct Sub;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("sup"))]
pub struct Sup;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("time"))]
pub struct Time;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("u"))]
pub struct U;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("var"))]
pub struct Var;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("wbr"))]
pub struct Wbr;

// Image and multimedia
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("area"))]
pub struct Area;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("audio"))]
pub struct Audio;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("img"))]
pub struct Img;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("map"))]
pub struct Map;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("track"))]
pub struct Track;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("video"))]
pub struct Video;

// Embedded content
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("embed"))]
pub struct Embed;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("fencedframe"))]
pub struct FencedFrame;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("iframe"))]
pub struct Iframe;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("object"))]
pub struct Object;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("picture"))]
pub struct Picture;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("source"))]
pub struct Source;

// Scripting
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("canvas"))]
pub struct Canvas;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("noscript"))]
pub struct NoScript;

// TODO: does this need special support?
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("script"))]
pub struct Script;

// Demarcating edits
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("del"))]
pub struct Del;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("ins"))]
pub struct Ins;

// Table content
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("caption"))]
pub struct Caption;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("col"))]
pub struct Col;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("colgroup"))]
pub struct ColGroup;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("table"))]
pub struct Table;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("tbody"))]
pub struct Tbody;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("td"))]
pub struct Td;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("tfoot"))]
pub struct Tfoot;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("th"))]
pub struct Th;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("thead"))]
pub struct Thead;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("tr"))]
pub struct Tr;

// Forms
#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("button"))]
pub struct Button;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("datalist"))]
pub struct DataList;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("fieldset"))]
pub struct FieldSet;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("form"))]
pub struct Form;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("input"))]
pub struct Input;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("label"))]
pub struct Label;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("legend"))]
pub struct Legend;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("meter"))]
pub struct Meter;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("optgroup"))]
pub struct OptGroup;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("option"))]
pub struct OptionElement;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("output"))]
pub struct Output;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("progress"))]
pub struct Progress;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("select"))]
pub struct Select;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("selectedcontent"))]
pub struct SelectedContent;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("textarea"))]
pub struct TextArea;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("h1"))]
pub struct H1;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("h2"))]
pub struct H2;

#[derive(Debug, Default, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("h3"))]
pub struct H3;
