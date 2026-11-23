use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

type Fields = IndexMap<String, Json>;

// the ResponsesAPIResponse fields a terminal event copies into the final update
pub const TERMINAL_FIELDS: &str = "usage reasoning tool_choice tools model instructions \
    temperature top_p max_output_tokens previous_response_id text truncation parallel_tool_calls \
    user store incomplete_details";

// the event types background_streaming.py branches on; every other type is a no-op
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollEvent {
    OutputItemAdded(Item),
    OutputItemDone(Item),
    // item_id and part
    ContentPartAdded(Option<String>, Json),
    // item_id, part and content_index
    ContentPartDone(Option<String>, Json, i64),
    // item_id, content_index and delta
    OutputTextDelta(Option<String>, i64, String),
    InProgress,
    Terminal(Terminal),
    Unknown,
}

// one streamed output item; an absent or null id never reaches the output map
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub id: Option<String>,
    pub value: IndexMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    pub kind: TerminalKind,
    pub status: Option<String>,
    pub error: Option<Json>,
    pub fields: IndexMap<String, Json>,
    pub output: Option<Vec<Item>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    Completed,
    Failed,
    Incomplete,
    Cancelled,
}

impl TerminalKind {
    // the source's _event_to_status map
    pub(super) const fn default_status(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Incomplete => "incomplete",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PollEventError {
    #[error("the streaming event is not a JSON object")]
    NotObject,
    #[error("the streaming event field {0} is not of the declared type")]
    Field(&'static str),
    #[error("the streaming event names content index {index}, which has no part")]
    ContentIndex { index: i64 },
}

impl PollEvent {
    pub fn decode(value: &Json) -> Result<Self, PollEventError> {
        let Json::Object(fields) = value else {
            return Err(PollEventError::NotObject);
        };
        let name = text_field(fields, "type")?.unwrap_or_default();
        if let Some(kind) = terminal_kind(&name) {
            return Ok(Self::Terminal(terminal_of(kind, fields)?));
        }
        match name.as_str() {
            "response.output_item.added" => Ok(Self::OutputItemAdded(item_of(fields)?)),
            "response.output_item.done" => Ok(Self::OutputItemDone(item_of(fields)?)),
            "response.content_part.added" => Ok(Self::ContentPartAdded(
                text_field(fields, "item_id")?,
                part_of(fields),
            )),
            "response.content_part.done" => Ok(Self::ContentPartDone(
                text_field(fields, "item_id")?,
                part_of(fields),
                index_of(fields)?,
            )),
            "response.output_text.delta" => Ok(Self::OutputTextDelta(
                text_field(fields, "item_id")?,
                index_of(fields)?,
                text_field(fields, "delta")?.unwrap_or_default(),
            )),
            "response.in_progress" => Ok(Self::InProgress),
            _ => Ok(Self::Unknown),
        }
    }
}

fn terminal_kind(name: &str) -> Option<TerminalKind> {
    match name {
        "response.completed" => Some(TerminalKind::Completed),
        "response.failed" => Some(TerminalKind::Failed),
        "response.incomplete" => Some(TerminalKind::Incomplete),
        "response.cancelled" => Some(TerminalKind::Cancelled),
        _ => None,
    }
}

// an absent or null member is absent; any other value is handed on as given
fn member<'fields>(fields: &'fields Fields, name: &str) -> Option<&'fields Json> {
    match fields.get(name) {
        None | Some(Json::Null) => None,
        Some(value) => Some(value),
    }
}

fn text_field(fields: &Fields, name: &'static str) -> Result<Option<String>, PollEventError> {
    match member(fields, name) {
        None => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PollEventError::Field(name)),
    }
}

fn item_of(fields: &Fields) -> Result<Item, PollEventError> {
    let Some(value) = member(fields, "item") else {
        return Ok(Item::default());
    };
    item_value(value, "item")
}

fn item_value(value: &Json, field: &'static str) -> Result<Item, PollEventError> {
    let Json::Object(item) = value else {
        return Err(PollEventError::Field(field));
    };
    match item.get("content") {
        None | Some(Json::Null | Json::List(_)) => {}
        Some(_) => return Err(PollEventError::Field("content")),
    }
    Ok(Item {
        id: text_field(item, "id")?,
        value: item.clone(),
    })
}

fn index_of(fields: &Fields) -> Result<i64, PollEventError> {
    match member(fields, "content_index") {
        None => Ok(0),
        Some(Json::Number(number)) => number
            .as_i64()
            .ok_or(PollEventError::Field("content_index")),
        Some(_) => Err(PollEventError::Field("content_index")),
    }
}

// the source's default for an absent or null part is {}
fn part_of(fields: &Fields) -> Json {
    member(fields, "part")
        .cloned()
        .unwrap_or_else(|| Json::Object(IndexMap::new()))
}

fn terminal_of(kind: TerminalKind, fields: &Fields) -> Result<Terminal, PollEventError> {
    let response = match member(fields, "response") {
        None => IndexMap::new(),
        Some(Json::Object(response)) => response.clone(),
        Some(_) => return Err(PollEventError::Field("response")),
    };
    let output = match member(&response, "output") {
        None => None,
        Some(Json::List(items)) => Some(
            items
                .iter()
                .map(|item| item_value(item, "response.output item"))
                .collect::<Result<Vec<Item>, PollEventError>>()?,
        ),
        Some(_) => return Err(PollEventError::Field("response.output")),
    };
    // a failed or incomplete event rewrites the error, so an absent one clears an earlier failure
    let error = if matches!(kind, TerminalKind::Failed | TerminalKind::Incomplete) {
        let error = response.get("error").cloned().unwrap_or(Json::Null);
        if !matches!(error, Json::Null | Json::Object(_)) {
            return Err(PollEventError::Field("response.error"));
        }
        Some(error)
    } else {
        None
    };
    Ok(Terminal {
        kind,
        status: text_field(&response, "status")?,
        error,
        fields: terminal_fields(&response),
        output,
    })
}

fn terminal_fields(response: &Fields) -> Fields {
    let mut fields = IndexMap::with_capacity(TERMINAL_FIELDS.split(' ').count());
    for name in TERMINAL_FIELDS.split(' ') {
        let value = response.get(name).cloned().unwrap_or(Json::Null);
        fields.insert(name.to_owned(), value);
    }
    fields
}
