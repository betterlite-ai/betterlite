use quick_xml::Reader;
use quick_xml::encoding::EncodingError;
use quick_xml::escape::{EscapeError, unescape};
use quick_xml::events::Event;
use thiserror::Error;

use super::date_parse::parse_date;

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

#[derive(Default)]
struct Item {
    title: Option<String>,
    link: Option<String>,
    description: Option<String>,
    pub_date: Option<String>,
}

impl Item {
    // Mirrors `Element.find()`: only the first direct child with this tag name counts.
    fn record(&mut self, tag: &str, text: String) {
        let field = match tag {
            TITLE => &mut self.title,
            LINK => &mut self.link,
            DESCRIPTION => &mut self.description,
            PUB_DATE => &mut self.pub_date,
            _ => return,
        };
        if field.is_none() {
            *field = Some(text);
        }
    }

    fn finish(self) -> Option<BlogPost> {
        let title = self.title?;
        let url = self.link?;
        Some(BlogPost {
            title,
            description: self.description.unwrap_or_default(),
            date: self
                .pub_date
                .map_or_else(String::new, |raw| parse_date(&raw)),
            url,
        })
    }
}

struct ActiveField {
    tag: String,
    depth: usize,
    text: String,
    saw_child: bool,
}

struct Collector {
    posts: Vec<BlogPost>,
    max_posts: usize,
    channel_seen: bool,
    channel_depth: Option<usize>,
    channel_open: bool,
    item: Option<(usize, Item)>,
    active_field: Option<ActiveField>,
    depth: usize,
    root_count: usize,
    root_closed: bool,
    default_namespace: Vec<bool>,
}

impl Collector {
    fn open(&mut self, name: &str, qualified: bool, default_namespace: bool) {
        if self.depth == 0 {
            if self.root_count > 0 {
                self.root_count = self.root_count.saturating_add(1);
                return;
            }
            self.root_count = 1;
            self.root_closed = false;
        }
        self.depth = self.depth.saturating_add(1);
        self.default_namespace.push(default_namespace);
        if let Some(active) = &mut self.active_field {
            if self.depth > active.depth {
                active.saw_child = true;
            }
        }
        // Mirrors `root.find("channel")`: only a direct child of the document root counts.
        if !qualified && name == CHANNEL && self.depth == 2 && self.channel_depth.is_none() {
            self.channel_depth = Some(self.depth);
            self.channel_seen = true;
            self.channel_open = true;
        }
        // Mirrors `channel.findall("item")`: only direct children of channel count.
        let is_direct_channel_child = self
            .channel_depth
            .is_some_and(|depth| self.channel_open && self.depth == depth.saturating_add(1));
        if !qualified && name == ITEM && is_direct_channel_child && self.item.is_none() {
            self.item = Some((self.depth, Item::default()));
            return;
        }
        if let Some((item_depth, _)) = &self.item {
            let is_field = !qualified && matches!(name, TITLE | LINK | DESCRIPTION | PUB_DATE);
            if is_field && self.depth == item_depth.saturating_add(1) && self.active_field.is_none()
            {
                self.active_field = Some(ActiveField {
                    tag: name.to_owned(),
                    depth: self.depth,
                    text: String::new(),
                    saw_child: false,
                });
            }
        }
    }

    fn push_text(&mut self, value: &str) -> Result<(), ParseFailure> {
        if self.depth == 0 && !value.trim().is_empty() {
            return Err(ParseFailure::OutsideRoot);
        }
        let Some(active) = &mut self.active_field else {
            return Ok(());
        };
        if self.depth == active.depth && !active.saw_child {
            active.text.push_str(value);
        }
        Ok(())
    }

    fn close(&mut self, name: &str, qualified: bool) {
        if !qualified {
            self.close_active_field(name);
            self.close_item(name);
            self.close_channel(name);
        }
        self.depth = self.depth.saturating_sub(1);
        let _ = self.default_namespace.pop();
        if self.depth == 0 {
            self.root_closed = true;
        }
        let _ = qualified;
    }

    fn close_active_field(&mut self, name: &str) {
        let closes_now = self
            .active_field
            .as_ref()
            .is_some_and(|active| self.depth == active.depth && active.tag == name);
        if !closes_now {
            return;
        }
        let Some(finished) = self.active_field.take() else {
            return;
        };
        let Some((_, item)) = &mut self.item else {
            return;
        };
        item.record(&finished.tag, finished.text);
    }

    fn close_item(&mut self, name: &str) {
        let closes_now = self
            .item
            .as_ref()
            .is_some_and(|(item_depth, _)| self.depth == *item_depth);
        if !(closes_now && name == ITEM) {
            return;
        }
        let Some((_, item)) = self.item.take() else {
            return;
        };
        if self.posts.len() >= self.max_posts {
            return;
        }
        if let Some(post) = item.finish() {
            self.posts.push(post);
        }
    }

    fn close_channel(&mut self, name: &str) {
        if self.channel_depth == Some(self.depth) && name == CHANNEL {
            self.channel_open = false;
        }
    }

    fn finish(self) -> Result<Vec<BlogPost>, ParseFailure> {
        if self.depth != 0 || self.root_count != 1 || !self.root_closed {
            return Err(ParseFailure::UnclosedElement);
        }
        if self.channel_seen {
            Ok(self.posts)
        } else {
            Err(ParseFailure::MissingChannel)
        }
    }
}

// Validates the whole document, matching `ElementTree`'s full-document parse.
pub(super) fn parse_rss_to_posts(
    xml: &str,
    max_posts: usize,
) -> Result<Vec<BlogPost>, ParseFailure> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().check_end_names = true;
    let mut collector = Collector {
        posts: Vec::new(),
        max_posts,
        channel_seen: false,
        channel_depth: None,
        channel_open: false,
        item: None,
        active_field: None,
        depth: 0,
        root_count: 0,
        root_closed: false,
        default_namespace: vec![false],
    };
    run_events(&mut reader, &mut collector)?;
    collector.finish()
}

fn run_events(reader: &mut Reader<&[u8]>, collector: &mut Collector) -> Result<(), ParseFailure> {
    loop {
        match reader.read_event().map_err(ParseFailure::Xml)? {
            Event::Start(tag) => {
                let (qualified, default_namespace) = tag_info(collector, &tag)?;
                collector.open(
                    &String::from_utf8_lossy(tag.name().as_ref()),
                    qualified,
                    default_namespace,
                );
            }
            Event::End(tag) => {
                let name = String::from_utf8_lossy(tag.name().as_ref());
                let qualified = name.contains(':')
                    || collector.default_namespace.last().copied().unwrap_or(false);
                collector.close(&name, qualified);
            }
            Event::Empty(tag) => {
                let name = String::from_utf8_lossy(tag.name().as_ref()).into_owned();
                let (qualified, default_namespace) = tag_info(collector, &tag)?;
                collector.open(&name, qualified, default_namespace);
                collector.close(&name, qualified);
            }
            Event::Text(text) => {
                let decoded = text.decode().map_err(ParseFailure::Decode)?;
                let value = unescape(decoded.as_ref()).map_err(ParseFailure::Escape)?;
                collector.push_text(&value)?;
            }
            Event::CData(text) => {
                let decoded = text.decode().map_err(ParseFailure::Decode)?;
                collector.push_text(&decoded)?;
            }
            Event::GeneralRef(reference) => {
                let decoded = reference.decode().map_err(ParseFailure::Decode)?;
                let wrapped = format!("&{decoded};");
                let value = unescape(&wrapped).map_err(ParseFailure::Escape)?;
                collector.push_text(&value)?;
            }
            Event::Eof => return Ok(()),
            Event::Decl(_) | Event::PI(_) | Event::DocType(_) | Event::Comment(_) => {}
        }
    }
}

fn tag_info(
    collector: &Collector,
    tag: &quick_xml::events::BytesStart<'_>,
) -> Result<(bool, bool), ParseFailure> {
    let inherited = collector.default_namespace.last().copied().unwrap_or(false);
    let declaration = tag
        .try_get_attribute(b"xmlns")
        .map_err(|error| ParseFailure::Xml(error.into()))?;
    let default_namespace = declaration
        .as_ref()
        .map_or(inherited, |attribute| !attribute.value.is_empty());
    let qualified = tag.name().as_ref().contains(&b':') || default_namespace;
    Ok((qualified, default_namespace))
}

#[path = "parser/tests.rs"]
mod tests;
