use super::HtmlElementName;
use bevy_ecs::prelude::*;

// Main Root
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("html"))]
pub struct Html;

// Document metadata
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("base"))]
pub struct Base;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("head"))]
pub struct Head;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("link"))]
pub struct Link;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("meta"))]
pub struct Meta;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("style"))]
pub struct StyleElement;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("title"))]
pub struct TitleElement;

// Sectioning root
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("body"))]
pub struct Body;

// Content sectioning
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("address"))]
pub struct Address;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("article"))]
pub struct Article;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("aside"))]
pub struct Aside;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("footer"))]
pub struct Footer;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("header"))]
pub struct Header;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("hgroup"))]
pub struct Hgroup;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("main"))]
pub struct Main;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("nav"))]
pub struct Nav;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("section"))]
pub struct Section;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("search"))]
pub struct Search;

// Text content
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("blockquote"))]
pub struct BlockQuote;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("dd"))]
pub struct Dd;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("div"))]
pub struct Div;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("dl"))]
pub struct Dl;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("dt"))]
pub struct Dt;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("figcaption"))]
pub struct FigCaption;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("figure"))]
pub struct Figure;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("hr"))]
pub struct Hr;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("li"))]
pub struct Li;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("menu"))]
pub struct Menu;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("ol"))]
pub struct Ol;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("p"))]
pub struct P;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("pre"))]
pub struct Pre;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("ul"))]
pub struct Ul;

// Inline text semantics
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("a"))]
pub struct A;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("abbr"))]
pub struct Abbr;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("b"))]
pub struct B;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("bdi"))]
pub struct Bdi;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("bdo"))]
pub struct Bdo;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("br"))]
pub struct Br;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("cite"))]
pub struct Cite;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("code"))]
pub struct Code;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("data"))]
pub struct Data;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("dfn"))]
pub struct Dfn;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("em"))]
pub struct Em;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("i"))]
pub struct I;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("kbd"))]
pub struct Kbd;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("mark"))]
pub struct Mark;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("q"))]
pub struct Q;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("rp"))]
pub struct Rp;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("rt"))]
pub struct Rt;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("ruby"))]
pub struct Ruby;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("s"))]
pub struct S;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("samp"))]
pub struct Samp;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("small"))]
pub struct Small;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("span"))]
pub struct Span;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("strong"))]
pub struct Strong;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("sub"))]
pub struct Sub;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("sup"))]
pub struct Sup;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("time"))]
pub struct Time;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("u"))]
pub struct U;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("var"))]
pub struct Var;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("wbr"))]
pub struct Wbr;

// Image and multimedia
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("area"))]
pub struct Area;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("audio"))]
pub struct Audio;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("img"))]
pub struct Img;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("map"))]
pub struct Map;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("track"))]
pub struct Track;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("video"))]
pub struct Video;

// Embedded content
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("embed"))]
pub struct Embed;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("fencedframe"))]
pub struct FencedFrame;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("iframe"))]
pub struct Iframe;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("object"))]
pub struct Object;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("picture"))]
pub struct Picture;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("source"))]
pub struct Source;

// Scripting
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("canvas"))]
pub struct Canvas;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("noscript"))]
pub struct NoScript;

// TODO: does this need special support?
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("script"))]
pub struct Script;

// Demarcating edits
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("del"))]
pub struct Del;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("ins"))]
pub struct Ins;

// Table content
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("caption"))]
pub struct Caption;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("col"))]
pub struct Col;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("colgroup"))]
pub struct ColGroup;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("table"))]
pub struct Table;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("tbody"))]
pub struct Tbody;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("td"))]
pub struct Td;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("tfoot"))]
pub struct Tfoot;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("th"))]
pub struct Th;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("thead"))]
pub struct Thead;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("tr"))]
pub struct Tr;

// Forms
#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("button"))]
pub struct Button;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("datalist"))]
pub struct DataList;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("fieldset"))]
pub struct FieldSet;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("form"))]
pub struct Form;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("input"))]
pub struct Input;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("label"))]
pub struct Label;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("legend"))]
pub struct Legend;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("meter"))]
pub struct Meter;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("optgroup"))]
pub struct OptGroup;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("option"))]
pub struct OptionElement;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("output"))]
pub struct Output;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("progress"))]
pub struct Progress;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("select"))]
pub struct Select;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("selectedcontent"))]
pub struct SelectedContent;

#[derive(Debug, Component, Clone, PartialEq, Eq)]
#[require(HtmlElementName("textarea"))]
pub struct TextArea;
