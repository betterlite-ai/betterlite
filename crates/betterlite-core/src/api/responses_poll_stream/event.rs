use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

// the ResponsesAPIResponse fields a terminal event copies into the final update
pub const TERMINAL_FIELDS: [&str; 16] = [
    "usage",
    "reasoning",
    "tool_choice",
    "tools",
    "model",
    "instructions",
    "temperature",
    "top_p",
    "max_output_tokens",
    "previous_response_id",
    "text",
    "truncation",
    "parallel_tool_calls",
    "user",
    "store",
    "incomplete_details",
];

// the event types background_streaming.py branches on; every other type is a no-op
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollEvent {
    OutputItemAdded(Item),
    OutputItemDone(Item),
    ContentPartAdded {
        item_id: Option<String>,
        part: Json,
    },
    ContentPartDone {
        item_id: Option<String>,
        part: Json,
        content_index: i64,
    },
    OutputTextDelta {
        item_id: Option<String>,
        content_index: i64,
        delta: String,
    },
    InProgress,
    Terminal(Terminal),
    Unknown,
}

// one streamed output item; an absent or empty id never reaches the output map
#[derive(Debug, Clone, PartialEq, Eq)]
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

    // only a failed or incomplete response carries response.error into the record
    const fn captures_error(self) -> bool {
        matches!(self, Self::Failed | Self::Incomplete)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PollEventError {
    #[error("the streaming event is not a JSON object")]
    NotObject,
    #[error("the streaming event field {field} is not of the declared type")]
    Field { field: &'static str },
    #[error("the streaming event names content index {index}, which has no part")]
    ContentIndex { index: i64 },
}

impl PollEvent {
    pub fn decode(value: &Json) -> Result<Self, PollEventError> {
        let Json::Object(fields) = value else {
            return Err(PollEventError::NotObject);
        };
        let name = match fields.get("type") {
            None | Some(Json::Null) => return Ok(Self::Unknown),
            Some(Json::Text(name)) => name,
            Some(_) => return Err(PollEventError::Field { field: "type" }),
        };
        match name.as_str() {
            "response.output_item.added" => Ok(Self::OutputItemAdded(item_of(fields)?)),
            "response.output_item.done" => Ok(Self::OutputItemDone(item_of(fields)?)),
            "response.content_part.added" => Ok(Self::ContentPartAdded {
                item_id: item_id_of(fields)?,
                part: part_of(fields),
            }),
            "response.content_part.done" => Ok(Self::ContentPartDone {
                item_id: item_id_of(fields)?,
                part: part_of(fields),
                content_index: index_of(fields)?,
            }),
            "response.output_text.delta" => Ok(Self::OutputTextDelta {
                item_id: item_id_of(fields)?,
                content_index: index_of(fields)?,
                delta: text_of(fields)?,
            }),
            "response.in_progress" => Ok(Self::InProgress),
            "response.completed" => Ok(Self::Terminal(terminal_of(
                TerminalKind::Completed,
                fields,
            )?)),
            "response.failed" => Ok(Self::Terminal(terminal_of(TerminalKind::Failed, fields)?)),
            "response.incomplete" => Ok(Self::Terminal(terminal_of(
                TerminalKind::Incomplete,
                fields,
            )?)),
            "response.cancelled" => Ok(Self::Terminal(terminal_of(
                TerminalKind::Cancelled,
                fields,
            )?)),
            _ => Ok(Self::Unknown),
        }
    }
}

fn item_of(fields: &IndexMap<String, Json>) -> Result<Item, PollEventError> {
    match fields.get("item") {
        None | Some(Json::Null) => Ok(Item {
            id: None,
            value: IndexMap::new(),
        }),
        Some(value) => item_value(value, "item"),
    }
}

fn item_value(value: &Json, field: &'static str) -> Result<Item, PollEventError> {
    let Json::Object(item) = value else {
        return Err(PollEventError::Field { field });
    };
    match item.get("content") {
        None | Some(Json::Null | Json::List(_)) => {}
        Some(_) => return Err(PollEventError::Field { field: "content" }),
    }
    Ok(Item {
        id: id_of(item, field)?,
        value: item.clone(),
    })
}

// an absent or null id leaves no id; a given id is used as it is, empty or not
fn id_of(
    item: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, PollEventError> {
    match item.get("id") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(PollEventError::Field { field }),
    }
}

fn item_id_of(fields: &IndexMap<String, Json>) -> Result<Option<String>, PollEventError> {
    id_of(fields, "item_id")
}

fn index_of(fields: &IndexMap<String, Json>) -> Result<i64, PollEventError> {
    match fields.get("content_index") {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(PollEventError::Field {
            field: "content_index",
        }),
        Some(_) => Err(PollEventError::Field {
            field: "content_index",
        }),
    }
}

fn text_of(fields: &IndexMap<String, Json>) -> Result<String, PollEventError> {
    match fields.get("delta") {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(_) => Err(PollEventError::Field { field: "delta" }),
    }
}

// the source's default for an absent or null part is {}
fn part_of(fields: &IndexMap<String, Json>) -> Json {
    match fields.get("part") {
        None | Some(Json::Null) => Json::Object(IndexMap::new()),
        Some(part) => part.clone(),
    }
}

fn terminal_of(
    kind: TerminalKind,
    fields: &IndexMap<String, Json>,
) -> Result<Terminal, PollEventError> {
    let response = match fields.get("response") {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(response)) => response.clone(),
        Some(_) => return Err(PollEventError::Field { field: "response" }),
    };
    let status = match response.get("status") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(_) => {
            return Err(PollEventError::Field {
                field: "response.status",
            });
        }
    };
    let output = match response.get("output") {
        None | Some(Json::Null) => None,
        Some(Json::List(items)) => Some(
            items
                .iter()
                .map(|item| item_value(item, "response.output item"))
                .collect::<Result<Vec<Item>, PollEventError>>()?,
        ),
        Some(_) => {
            return Err(PollEventError::Field {
                field: "response.output",
            });
        }
    };
    // a failed or incomplete event rewrites the error, so an absent one clears an earlier failure
    let error = if kind.captures_error() {
        let error = response.get("error").cloned().unwrap_or(Json::Null);
        if !matches!(error, Json::Null | Json::Object(_)) {
            return Err(PollEventError::Field {
                field: "response.error",
            });
        }
        Some(error)
    } else {
        None
    };
    Ok(Terminal {
        kind,
        status,
        error,
        fields: terminal_fields(&response),
        output,
    })
}

fn terminal_fields(response: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut fields = IndexMap::with_capacity(TERMINAL_FIELDS.len());
    for name in TERMINAL_FIELDS {
        let value = response.get(name).cloned().unwrap_or(Json::Null);
        fields.insert(name.to_owned(), value);
    }
    fields
}
