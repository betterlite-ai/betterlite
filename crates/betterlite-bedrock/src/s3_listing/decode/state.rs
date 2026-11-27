use quick_xml::events::{BytesRef, BytesStart};

use super::{DecodeLimits, S3ListedEntry, S3Listing};
use crate::s3_listing::S3ListingError;

// ElementTree's findtext takes the first matching child even when its text is empty
#[derive(Default)]
enum FirstText {
    #[default]
    Unseen,
    Seen(Option<String>),
}

impl FirstText {
    fn value(self) -> Option<String> {
        match self {
            Self::Unseen => None,
            Self::Seen(value) => value,
        }
    }

    fn set(&mut self, text: String) {
        if matches!(self, Self::Unseen) {
            *self = Self::Seen(nonempty(text));
        }
    }
}

fn nonempty(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

#[derive(Debug, Clone, Copy)]
enum Field {
    Key,
    Size,
    LastModified,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Root,
    Contents { entry: usize },
    Name,
    Token,
    Field { entry: usize, field: Field },
    Other,
}

pub(super) struct Frame {
    kind: Kind,
    text: String,
    child_seen: bool,
}

#[derive(Default)]
struct Draft {
    key: FirstText,
    size: FirstText,
    last_modified: FirstText,
}

impl Draft {
    fn into_entry(self) -> S3ListedEntry {
        S3ListedEntry {
            key: self.key.value(),
            size: self.size.value(),
            last_modified: self.last_modified.value(),
        }
    }
}

#[derive(Default)]
pub(super) struct ListingBuilder {
    pub(super) stack: Vec<Frame>,
    pub(super) root_seen: bool,
    name: FirstText,
    token: FirstText,
    drafts: Vec<Draft>,
}

fn child_kind(local: &[u8], drafts: &mut Vec<Draft>) -> Kind {
    match local {
        b"Name" => Kind::Name,
        b"NextContinuationToken" => Kind::Token,
        b"Contents" => {
            let entry = drafts.len();
            drafts.push(Draft::default());
            Kind::Contents { entry }
        }
        _ => Kind::Other,
    }
}

fn field_kind(local: &[u8], entry: usize) -> Kind {
    match local {
        b"Key" => Kind::Field {
            entry,
            field: Field::Key,
        },
        b"Size" => Kind::Field {
            entry,
            field: Field::Size,
        },
        b"LastModified" => Kind::Field {
            entry,
            field: Field::LastModified,
        },
        _ => Kind::Other,
    }
}

fn finish(builder: &mut ListingBuilder, frame: Frame) {
    match frame.kind {
        Kind::Name => builder.name.set(frame.text),
        Kind::Token => builder.token.set(frame.text),
        Kind::Field { entry, field } => {
            let Some(draft) = builder.drafts.get_mut(entry) else {
                return;
            };
            match field {
                Field::Key => draft.key.set(frame.text),
                Field::Size => draft.size.set(frame.text),
                Field::LastModified => draft.last_modified.set(frame.text),
            }
        }
        Kind::Root | Kind::Contents { .. } | Kind::Other => {}
    }
}

pub(super) fn open(
    builder: &mut ListingBuilder,
    start: &BytesStart<'_>,
    limits: &DecodeLimits,
) -> Result<(), S3ListingError> {
    if builder.stack.len() >= limits.depth {
        return Err(S3ListingError::DepthExceeded {
            limit: limits.depth,
        });
    }
    if let Some(parent) = builder.stack.last_mut() {
        parent.child_seen = true;
    }
    let depth = builder.stack.len().saturating_add(1);
    let local = start.local_name();
    let kind = if depth == 1 {
        if builder.root_seen {
            return Err(S3ListingError::Trailing);
        }
        builder.root_seen = true;
        Kind::Root
    } else if depth == 2 {
        child_kind(local.as_ref(), &mut builder.drafts)
    } else if depth == 3 {
        // iterfind("{*}Contents") visits direct children only
        match builder.stack.last().map(|frame| frame.kind) {
            Some(Kind::Contents { entry }) => field_kind(local.as_ref(), entry),
            Some(Kind::Root | Kind::Name | Kind::Token | Kind::Field { .. } | Kind::Other)
            | None => Kind::Other,
        }
    } else {
        Kind::Other
    };
    builder.stack.push(Frame {
        kind,
        text: String::new(),
        child_seen: false,
    });
    Ok(())
}

pub(super) fn close(builder: &mut ListingBuilder) -> Result<(), S3ListingError> {
    let Some(frame) = builder.stack.pop() else {
        return Err(S3ListingError::Trailing);
    };
    finish(builder, frame);
    Ok(())
}

pub(super) fn open_empty(
    builder: &mut ListingBuilder,
    start: &BytesStart<'_>,
    limits: &DecodeLimits,
) -> Result<(), S3ListingError> {
    open(builder, start, limits)?;
    close(builder)
}

pub(super) fn text(builder: &mut ListingBuilder, content: &str) -> Result<(), S3ListingError> {
    let Some(frame) = builder.stack.last_mut() else {
        if content.chars().all(char::is_whitespace) {
            return Ok(());
        }
        return Err(S3ListingError::Trailing);
    };
    if !frame.child_seen {
        frame.text.push_str(content);
    }
    Ok(())
}

pub(super) fn reference(
    builder: &mut ListingBuilder,
    node: &BytesRef<'_>,
) -> Result<(), S3ListingError> {
    if let Some(character) = node.resolve_char_ref()? {
        let mut encoded = [0_u8; 4];
        let unit: &str = character.encode_utf8(&mut encoded);
        return text(builder, unit);
    }
    let name = node.decode()?.into_owned();
    match name.as_str() {
        "amp" => text(builder, "&"),
        "lt" => text(builder, "<"),
        "gt" => text(builder, ">"),
        "apos" => text(builder, "'"),
        "quot" => text(builder, "\""),
        _ => Err(S3ListingError::Entity { name }),
    }
}

pub(super) fn into_listing(builder: ListingBuilder) -> S3Listing {
    S3Listing {
        name: builder.name.value(),
        contents: builder.drafts.into_iter().map(Draft::into_entry).collect(),
        next_continuation_token: builder.token.value(),
    }
}
