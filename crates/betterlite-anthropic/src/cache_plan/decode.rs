use indexmap::IndexMap;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::cache_plan::json::{JsonError, compact};
use crate::cache_plan::wire::{Block, CacheControl, Content, PlanRequest, Role, Speed, Ttl};

type Object<'body> = IndexMap<String, &'body RawValue>;
pub(super) type Fields = IndexMap<String, Box<RawValue>>;

const PLAN_ONLY_OPTIONS: [&str; 6] = [
    "cache_control",
    "thinking",
    "tool_choice",
    "output_config",
    "speed",
    "service_tier",
];

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Section {
    Tools,
    System,
    Messages,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Thinking,
    BlankText,
    ToolUse,
    ToolResult,
    Other,
}

pub(super) struct Position {
    pub(super) section: Section,
    pub(super) message_index: i64,
    pub(super) role: String,
    pub(super) kind: Kind,
    pub(super) fields: Fields,
    pub(super) marker: Option<Ttl>,
    // a null cache_control still counts, since the prediction schema forbids the key itself
    pub(super) controlled: bool,
}

pub(super) struct Decoded {
    pub(super) full_body: Box<RawValue>,
    pub(super) cache_control: Option<Ttl>,
    pub(super) thinking: Option<Box<RawValue>>,
    pub(super) tool_choice: Option<Box<RawValue>>,
    pub(super) output_config: Option<Box<RawValue>>,
    pub(super) speed: Option<Box<RawValue>>,
    pub(super) predictable: bool,
    pub(super) positions: Vec<Position>,
}

enum Source<'body> {
    Text(&'body RawValue),
    Object(Object<'body>),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum DecodeError {
    #[error("the request is not a cache plan request")]
    Json(#[from] serde_json::Error),
    #[error("a request value cannot be written as compact JSON")]
    Compact(#[from] JsonError),
    #[error("the request has no messages")]
    NoMessages,
}

pub(super) fn decode(body: &str) -> Result<Decoded, DecodeError> {
    let request: PlanRequest = serde_json::from_str(body)?;
    if request.messages.is_empty() {
        return Err(DecodeError::NoMessages);
    }
    let top: Object<'_> = serde_json::from_str(body)?;
    let positions = positions(&top)?;
    let option = |name: &str| top.get(name).copied().map(compact).transpose();
    Ok(Decoded {
        full_body: serde_json::from_str(body)?,
        cache_control: request
            .cache_control
            .map(|CacheControl::Ephemeral { ttl }| ttl),
        thinking: option("thinking")?,
        tool_choice: option("tool_choice")?,
        output_config: option("output_config")?,
        speed: if request.speed == Some(Speed::Standard) {
            None
        } else {
            option("speed")?
        },
        predictable: predictable(&request, &top, &positions),
        positions,
    })
}

fn predictable(request: &PlanRequest, top: &Object<'_>, positions: &[Position]) -> bool {
    let plain_messages = request.messages.iter().all(|message| {
        message.role != Role::System
            && match &message.content {
                Content::Text(text) => !text.chars().all(char::is_whitespace),
                Content::Blocks(blocks) => !blocks
                    .iter()
                    .any(|block| matches!(block, Block::Thinking(_))),
            }
    });
    plain_messages
        && !PLAN_ONLY_OPTIONS.iter().any(|name| top.contains_key(*name))
        && !positions
            .iter()
            .any(|position| position.section != Section::Messages && position.controlled)
}

fn positions(top: &Object<'_>) -> Result<Vec<Position>, DecodeError> {
    let mut positions = Vec::new();
    for tool in objects(top.get("tools").copied())? {
        positions.push(position(Section::Tools, -1, "", Source::Object(tool))?);
    }
    for source in sources(top.get("system").copied())? {
        positions.push(position(Section::System, -1, "", source)?);
    }
    for (message_index, message) in (0_i64..).zip(objects(top.get("messages").copied())?) {
        let role: String = match message.get("role") {
            Some(raw) => serde_json::from_str(raw.get())?,
            None => String::new(),
        };
        for source in sources(message.get("content").copied())? {
            positions.push(position(Section::Messages, message_index, &role, source)?);
        }
    }
    Ok(positions)
}

fn objects(raw: Option<&RawValue>) -> Result<Vec<Object<'_>>, serde_json::Error> {
    match raw {
        Some(list) if list.get().starts_with('[') => serde_json::from_str(list.get()),
        Some(_) | None => Ok(Vec::new()),
    }
}

fn sources(raw: Option<&RawValue>) -> Result<Vec<Source<'_>>, serde_json::Error> {
    match raw {
        Some(text) if text.get().starts_with('"') => Ok(vec![Source::Text(text)]),
        Some(_) | None => Ok(objects(raw)?.into_iter().map(Source::Object).collect()),
    }
}

fn position(
    section: Section,
    message_index: i64,
    role: &str,
    source: Source<'_>,
) -> Result<Position, DecodeError> {
    let mut object = match source {
        Source::Text(text) => Object::from([
            ("type".to_owned(), serde_json::from_str(r#""text""#)?),
            ("text".to_owned(), text),
        ]),
        Source::Object(object) => object,
    };
    let control = object.shift_remove("cache_control");
    let block_type: Option<String> =
        serde_json::from_str(object.get("type").map_or("null", |raw| raw.get()))?;
    let text: Option<String> =
        serde_json::from_str(object.get("text").map_or("null", |raw| raw.get()))?;
    // Python's str.strip also drops the \x1c to \x1f separators that char::is_whitespace keeps
    let visible = text.is_some_and(|content| {
        content
            .chars()
            .any(|letter| !letter.is_whitespace() && !('\x1c'..='\x1f').contains(&letter))
    });
    let kind = match block_type.as_deref() {
        Some("thinking") => Kind::Thinking,
        Some("tool_use") => Kind::ToolUse,
        Some("tool_result") => Kind::ToolResult,
        Some("text") if !visible => Kind::BlankText,
        Some(_) | None => Kind::Other,
    };
    let marker: Option<CacheControl> =
        serde_json::from_str(control.map_or("null", |raw| raw.get()))?;
    let mut fields = Fields::with_capacity(object.len());
    for (key, raw) in object {
        fields.insert(key, compact(raw)?);
    }
    Ok(Position {
        section,
        message_index,
        role: role.to_owned(),
        kind,
        fields,
        marker: marker.map(|CacheControl::Ephemeral { ttl }| ttl),
        controlled: control.is_some(),
    })
}
