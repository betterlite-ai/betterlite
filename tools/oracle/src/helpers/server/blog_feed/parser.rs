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
}

struct Collector {
    posts: Vec<BlogPost>,
    max_posts: usize,
    channel_seen: bool,
    channel_depth: Option<usize>,
    item: Option<(usize, Item)>,
    active_field: Option<ActiveField>,
    depth: usize,
}

impl Collector {
    fn open(&mut self, name: &str) {
        self.depth = self.depth.saturating_add(1);
        // Mirrors `root.find("channel")`: only a direct child of the document root counts.
        if name == CHANNEL && self.depth == 2 && self.channel_depth.is_none() {
            self.channel_depth = Some(self.depth);
            self.channel_seen = true;
        }
        // Mirrors `channel.findall("item")`: only direct children of channel count.
        let is_direct_channel_child = self
            .channel_depth
            .is_some_and(|depth| self.depth == depth.saturating_add(1));
        if name == ITEM && is_direct_channel_child && self.item.is_none() {
            self.item = Some((self.depth, Item::default()));
            return;
        }
        if let Some((item_depth, _)) = &self.item {
            let is_field = matches!(name, TITLE | LINK | DESCRIPTION | PUB_DATE);
            if is_field && self.depth == item_depth.saturating_add(1) && self.active_field.is_none()
            {
                self.active_field = Some(ActiveField {
                    tag: name.to_owned(),
                    depth: self.depth,
                    text: String::new(),
                });
            }
        }
    }

    fn push_text(&mut self, value: &str) {
        let Some(active) = &mut self.active_field else {
            return;
        };
        if self.depth == active.depth {
            active.text.push_str(value);
        }
    }

    fn close(&mut self, name: &str) {
        self.close_active_field(name);
        self.close_item(name);
        self.close_channel(name);
        self.depth = self.depth.saturating_sub(1);
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
            self.channel_depth = None;
        }
    }

    fn finish(self) -> Result<Vec<BlogPost>, ParseFailure> {
        if self.depth != 0 {
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
    let mut collector = Collector {
        posts: Vec::new(),
        max_posts,
        channel_seen: false,
        channel_depth: None,
        item: None,
        active_field: None,
        depth: 0,
    };
    run_events(&mut reader, &mut collector)?;
    collector.finish()
}

fn run_events(reader: &mut Reader<&[u8]>, collector: &mut Collector) -> Result<(), ParseFailure> {
    loop {
        match reader.read_event().map_err(ParseFailure::Xml)? {
            Event::Start(tag) => {
                collector.open(&String::from_utf8_lossy(tag.name().as_ref()));
            }
            Event::End(tag) => {
                collector.close(&String::from_utf8_lossy(tag.name().as_ref()));
            }
            Event::Empty(tag) => {
                let name = String::from_utf8_lossy(tag.name().as_ref()).into_owned();
                collector.open(&name);
                collector.close(&name);
            }
            Event::Text(text) => {
                let decoded = text.decode().map_err(ParseFailure::Decode)?;
                let value = unescape(decoded.as_ref()).map_err(ParseFailure::Escape)?;
                collector.push_text(&value);
            }
            Event::CData(text) => {
                let decoded = text.decode().map_err(ParseFailure::Decode)?;
                collector.push_text(&decoded);
            }
            Event::GeneralRef(reference) => {
                let decoded = reference.decode().map_err(ParseFailure::Decode)?;
                let wrapped = format!("&{decoded};");
                let value = unescape(&wrapped).map_err(ParseFailure::Escape)?;
                collector.push_text(&value);
            }
            Event::Eof => return Ok(()),
            Event::Decl(_) | Event::PI(_) | Event::DocType(_) | Event::Comment(_) => {}
        }
    }
}

#[path = "parser/tests.rs"]
mod tests;
