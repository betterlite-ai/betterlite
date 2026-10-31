use quick_xml::Reader;
use quick_xml::encoding::EncodingError;
use quick_xml::escape::{EscapeError, unescape};
use quick_xml::events::Event;
use thiserror::Error;

use collector::{Collector, TagKind};

#[path = "parser/collector.rs"]
mod collector;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct BlogPost {
    pub(super) title: String,
    pub(super) description: String,
    pub(super) date: String,
    pub(super) url: String,
}

#[derive(Debug, Error)]
pub(super) enum ParseFailure {
    #[error("RSS XML is not well-formed")]
    Xml(#[source] quick_xml::Error),
    #[error("RSS item text is not valid UTF-8")]
    Decode(#[source] EncodingError),
    #[error("RSS item text has an invalid entity or character reference")]
    Escape(#[source] EscapeError),
    #[error("RSS feed missing <channel> element")]
    MissingChannel,
    #[error("RSS feed has an element that is never closed")]
    UnclosedElement,
    #[error("RSS XML has content outside its single root element")]
    OutsideRoot,
}

const TITLE: &str = "title";
const LINK: &str = "link";
const DESCRIPTION: &str = "description";
const PUB_DATE: &str = "pubDate";
const ITEM: &str = "item";
const CHANNEL: &str = "channel";

// Validates the whole document, matching `ElementTree`'s full-document parse.
pub(super) fn parse_rss_to_posts(
    xml: &str,
    max_posts: usize,
) -> Result<Vec<BlogPost>, ParseFailure> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().check_end_names = true;
    let mut collector = Collector::default();
    collector.max_posts = max_posts;
    run_events(&mut reader, &mut collector)?;
    collector.finish()
}

fn run_events(reader: &mut Reader<&[u8]>, collector: &mut Collector) -> Result<(), ParseFailure> {
    loop {
        match reader.read_event().map_err(ParseFailure::Xml)? {
            Event::Start(tag) => handle_start(&tag, collector)?,
            Event::End(tag) => handle_end(&tag, collector),
            Event::Empty(tag) => handle_empty(&tag, collector)?,
            Event::Text(text) => handle_text(&text, collector)?,
            Event::CData(text) => handle_cdata(&text, collector)?,
            Event::GeneralRef(reference) => handle_reference(&reference, collector)?,
            Event::Eof => return Ok(()),
            Event::Decl(_) | Event::PI(_) | Event::DocType(_) | Event::Comment(_) => {}
        }
    }
}

fn open_tag(
    tag: &quick_xml::events::BytesStart<'_>,
    collector: &mut Collector,
) -> Result<(String, TagKind), ParseFailure> {
    let (kind, default_namespace) = tag_info(collector, tag)?;
    let name = String::from_utf8_lossy(tag.name().as_ref()).into_owned();
    collector.open(&name, kind, default_namespace);
    Ok((name, kind))
}

fn handle_start(
    tag: &quick_xml::events::BytesStart<'_>,
    collector: &mut Collector,
) -> Result<(), ParseFailure> {
    open_tag(tag, collector)?;
    Ok(())
}

fn handle_end(tag: &quick_xml::events::BytesEnd<'_>, collector: &mut Collector) {
    let tag_name = tag.name().as_ref().to_owned();
    let name = String::from_utf8_lossy(&tag_name);
    let kind = if name.contains(':') || collector.default_namespace.last().copied().unwrap_or(false)
    {
        TagKind::Qualified
    } else {
        TagKind::Plain
    };
    collector.close(&name, kind);
}

fn handle_empty(
    tag: &quick_xml::events::BytesStart<'_>,
    collector: &mut Collector,
) -> Result<(), ParseFailure> {
    let (name, kind) = open_tag(tag, collector)?;
    collector.close(&name, kind);
    Ok(())
}

fn push_unescaped(raw: &str, collector: &mut Collector) -> Result<(), ParseFailure> {
    let value = unescape(raw).map_err(ParseFailure::Escape)?;
    collector.push_text(&value)
}

fn handle_text(
    text: &quick_xml::events::BytesText<'_>,
    collector: &mut Collector,
) -> Result<(), ParseFailure> {
    let decoded = text.decode().map_err(ParseFailure::Decode)?;
    push_unescaped(decoded.as_ref(), collector)
}

fn handle_cdata(
    text: &quick_xml::events::BytesCData<'_>,
    collector: &mut Collector,
) -> Result<(), ParseFailure> {
    let decoded = text.decode().map_err(ParseFailure::Decode)?;
    collector.push_text(&decoded)
}

fn handle_reference(
    reference: &quick_xml::events::BytesRef<'_>,
    collector: &mut Collector,
) -> Result<(), ParseFailure> {
    let decoded = reference.decode().map_err(ParseFailure::Decode)?;
    push_unescaped(&format!("&{decoded};"), collector)
}

fn tag_info(
    collector: &Collector,
    tag: &quick_xml::events::BytesStart<'_>,
) -> Result<(TagKind, bool), ParseFailure> {
    for attribute in tag.attributes() {
        attribute.map_err(|error| ParseFailure::Xml(error.into()))?;
    }
    let inherited = collector.default_namespace.last().copied().unwrap_or(false);
    let declaration = tag
        .try_get_attribute(b"xmlns")
        .map_err(|error| ParseFailure::Xml(error.into()))?;
    let default_namespace = declaration
        .as_ref()
        .map_or(inherited, |attribute| !attribute.value.is_empty());
    let tag_name = tag.name();
    let kind = if tag_name.as_ref().contains(&b':') || default_namespace {
        TagKind::Qualified
    } else {
        TagKind::Plain
    };
    Ok((kind, default_namespace))
}

#[path = "parser/tests.rs"]
mod tests;
