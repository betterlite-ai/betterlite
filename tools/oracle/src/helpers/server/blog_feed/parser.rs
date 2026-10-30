use super::{BlogPost, Failure};
use chrono::DateTime;
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesCData, BytesEnd, BytesStart, BytesText, Event};
use std::collections::BTreeMap;

enum ChannelState {
    Outside,
    Inside,
}

enum ItemState {
    Outside,
    Inside,
}

struct ParserState {
    posts: Vec<BlogPost>,
    fields: BTreeMap<String, String>,
    current: String,
    item_state: ItemState,
    channel_state: ChannelState,
    channel_seen: bool,
    max_posts: usize,
}

impl ParserState {
    fn start(&mut self, tag: &BytesStart<'_>) {
        let name = String::from_utf8_lossy(tag.name().as_ref()).into_owned();
        match name.as_str() {
            "channel" => {
                self.channel_state = ChannelState::Inside;
                self.channel_seen = true;
            }
            "item" if matches!(self.channel_state, ChannelState::Inside) => {
                self.item_state = ItemState::Inside;
                self.fields.clear();
            }
            _ => {}
        }
        self.current = name;
    }

    fn text(&mut self, text: &BytesText<'_>) -> Result<(), Failure> {
        if matches!(self.item_state, ItemState::Outside) {
            return Ok(());
        }
        let decoded = text.decode().map_err(|error| Failure::Parse {
            message: error.to_string(),
        })?;
        let value = unescape(decoded.as_ref()).map_err(|error| Failure::Parse {
            message: error.to_string(),
        })?;
        self.fields
            .entry(self.current.clone())
            .or_default()
            .push_str(&value);
        Ok(())
    }

    fn cdata(&mut self, text: &BytesCData<'_>) {
        if matches!(self.item_state, ItemState::Inside) {
            self.fields
                .entry(self.current.clone())
                .or_default()
                .push_str(&String::from_utf8_lossy(text.as_ref()));
        }
    }

    fn end(&mut self, tag: &BytesEnd<'_>) -> bool {
        let tag_name = tag.name();
        let name = String::from_utf8_lossy(tag_name.as_ref()).into_owned();
        match name.as_str() {
            "item" => {
                self.finish_item();
                self.item_state = ItemState::Outside;
            }
            "channel" => self.channel_state = ChannelState::Outside,
            _ => {}
        }
        self.current.clear();
        self.posts.len() >= self.max_posts
    }

    fn finish_item(&mut self) {
        let Some(title) = self.fields.get("title") else {
            return;
        };
        let Some(url) = self.fields.get("link") else {
            return;
        };
        if self.posts.len() >= self.max_posts {
            return;
        }
        self.posts.push(BlogPost {
            title: title.clone(),
            description: self.fields.get("description").cloned().unwrap_or_default(),
            date: parse_date(
                self.fields
                    .get("pubDate")
                    .map(String::as_str)
                    .unwrap_or_default(),
            ),
            url: url.clone(),
        });
    }
}

pub(super) fn parse_rss_to_posts(xml: &str, max_posts: usize) -> Result<Vec<BlogPost>, Failure> {
    let mut reader = Reader::from_str(xml);
    let mut state = ParserState {
        posts: Vec::new(),
        fields: BTreeMap::new(),
        current: String::new(),
        item_state: ItemState::Outside,
        channel_state: ChannelState::Outside,
        channel_seen: false,
        max_posts,
    };
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => state.start(&tag),
            Ok(Event::Text(text)) => state.text(&text)?,
            Ok(Event::CData(text)) => state.cdata(&text),
            Ok(Event::End(tag)) if state.end(&tag) => break,
            Ok(Event::Empty(tag)) if tag.name().as_ref() == b"channel" => {
                state.channel_state = ChannelState::Inside;
                state.channel_seen = true;
            }
            Ok(Event::Eof) => break,
            Ok(Event::End(_) | _) => {}
            Err(error) => {
                return Err(Failure::Parse {
                    message: error.to_string(),
                });
            }
        }
    }
    if state.channel_seen {
        Ok(state.posts)
    } else {
        Err(Failure::Parse {
            message: "RSS feed missing <channel> element".to_owned(),
        })
    }
}

fn parse_date(value: &str) -> String {
    let parsed = DateTime::parse_from_rfc2822(value);
    if parsed.is_err() {
        return value.to_owned();
    }
    parsed.map_or_else(
        |_| value.to_owned(),
        |date| date.format("%Y-%m-%d").to_string(),
    )
}
