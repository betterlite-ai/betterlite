use super::super::date_parse::parse_date;
use super::{BlogPost, CHANNEL, DESCRIPTION, ITEM, LINK, PUB_DATE, ParseFailure, TITLE};

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

#[derive(Clone, Copy)]
pub(super) enum TagKind {
    Plain,
    Qualified,
}

#[derive(Default)]
enum ChannelState {
    #[default]
    Unseen,
    Open(usize),
    Closed,
}

#[derive(Default)]
pub(super) struct Collector {
    posts: Vec<BlogPost>,
    pub(super) max_posts: usize,
    channel: ChannelState,
    item: Option<(usize, Item)>,
    active_field: Option<ActiveField>,
    depth: usize,
    root_count: usize,
    pub(super) default_namespace: Vec<bool>,
}

impl Collector {
    pub(super) fn open(&mut self, name: &str, kind: TagKind, default_namespace: bool) {
        if self.depth == 0 {
            if self.root_count > 0 {
                self.root_count = self.root_count.saturating_add(1);
                return;
            }
            self.root_count = 1;
        }
        self.depth = self.depth.saturating_add(1);
        self.default_namespace.push(default_namespace);
        if let Some(active) = &mut self.active_field
            && self.depth > active.depth
        {
            active.saw_child = true;
        }
        // Mirrors `root.find("channel")`: only a direct child of the document root counts.
        if matches!(kind, TagKind::Plain)
            && name == CHANNEL
            && self.depth == 2
            && matches!(self.channel, ChannelState::Unseen)
        {
            self.channel = ChannelState::Open(self.depth);
        }
        // Mirrors `channel.findall("item")`: only direct children of channel count.
        let is_direct_channel_child = matches!(
            self.channel,
            ChannelState::Open(depth) if self.depth == depth.saturating_add(1)
        );
        if matches!(kind, TagKind::Plain)
            && name == ITEM
            && is_direct_channel_child
            && self.item.is_none()
        {
            self.item = Some((self.depth, Item::default()));
            return;
        }
        self.open_field(name, kind);
    }

    fn open_field(&mut self, name: &str, kind: TagKind) {
        let Some((item_depth, _)) = &self.item else {
            return;
        };
        let is_field =
            matches!(kind, TagKind::Plain) && matches!(name, TITLE | LINK | DESCRIPTION | PUB_DATE);
        if is_field && self.depth == item_depth.saturating_add(1) && self.active_field.is_none() {
            self.active_field = Some(ActiveField {
                tag: name.to_owned(),
                depth: self.depth,
                text: String::new(),
                saw_child: false,
            });
        }
    }

    pub(super) fn push_text(&mut self, value: &str) -> Result<(), ParseFailure> {
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

    pub(super) fn close(&mut self, name: &str, kind: TagKind) {
        if matches!(kind, TagKind::Plain) {
            self.close_active_field(name);
            self.close_item(name);
            self.close_channel(name);
        }
        self.depth = self.depth.saturating_sub(1);
        self.default_namespace.pop();
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
        if matches!(self.channel, ChannelState::Open(depth) if depth == self.depth)
            && name == CHANNEL
            && let ChannelState::Open(_) = self.channel
        {
            self.channel = ChannelState::Closed;
        }
    }

    pub(super) fn finish(self) -> Result<Vec<BlogPost>, ParseFailure> {
        if self.depth != 0 || self.root_count != 1 {
            return Err(ParseFailure::UnclosedElement);
        }
        if matches!(self.channel, ChannelState::Unseen) {
            Err(ParseFailure::MissingChannel)
        } else {
            Ok(self.posts)
        }
    }
}
