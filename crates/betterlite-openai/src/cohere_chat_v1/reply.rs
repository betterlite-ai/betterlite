use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use serde::Serialize;
use serde_json::Error as JsonError;
use thiserror::Error;

use crate::cohere_streams::CohereUsage;
use crate::error::ProviderError;

use super::error::CohereError;

type Object = indexmap::IndexMap<String, Json>;

#[derive(Debug, Error)]
pub enum ReplyError {
    #[error("the Cohere response is not valid JSON: {1}")]
    Json(u16, #[source] JsonError),
    #[error("the Cohere response has no text")]
    Text(u16),
    #[error("the Cohere {0} field {1}")]
    Field(&'static str, &'static str),
    #[error("the Cohere token usage exceeds the supported range")]
    UsageOverflow,
}

impl ReplyError {
    // the JSON parse and the text lookup both fail with the provider's own status
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Json(status, _) | Self::Text(status) => Some(*status),
            Self::Field(..) | Self::UsageOverflow => None,
        }
    }
}

impl From<ReplyError> for ProviderError {
    // get_error_class builds the v1 CohereError, which drops the headers it was given
    fn from(error: ReplyError) -> Self {
        Self::from(CohereError {
            status_code: error.status_code().unwrap_or(500),
            message: error.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CohereV1Reply {
    pub id: String,
    pub created: u64,
    pub model: String,
    pub finish_reason: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<V1ToolCall>>,
    pub citations: Option<Json>,
    pub usage: CohereUsage,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct V1ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: V1Function,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct V1Function {
    pub name: Option<String>,
    pub arguments: String,
}

pub struct ReplyInput<'request> {
    pub body: &'request [u8],
    pub status: u16,
    pub model: &'request str,
    pub created: u64,
    pub id: String,
    pub finish_reason: Option<String>,
}

pub fn decode_cohere_chat_v1(input: ReplyInput<'_>) -> Result<CohereV1Reply, ReplyError> {
    let decoded: Json = serde_json::from_slice(input.body)
        .map_err(|source| ReplyError::Json(input.status, source))?;
    let Json::Object(fields) = &decoded else {
        return Err(ReplyError::Text(input.status));
    };
    // text is read before any tool, so a tools-only reply fails with the provider's own status
    let Some(text) = fields.get("text") else {
        return Err(ReplyError::Text(input.status));
    };
    let text_content = match text {
        Json::Null => None,
        Json::Text(value) => Some(value.clone()),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(ReplyError::Field("text", "is not a string"));
        }
    };
    let (content, tool_calls) = match fields.get("tool_calls") {
        Some(Json::List(tools)) if !tools.is_empty() => (
            None,
            Some(
                tools
                    .iter()
                    .map(reply_tool_call)
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        ),
        Some(Json::List(_) | Json::Null) | None => (text_content, None),
        Some(_) => return Err(ReplyError::Field("tool_calls", "is not a list")),
    };
    Ok(CohereV1Reply {
        id: input.id,
        created: input.created,
        model: input.model.to_owned(),
        finish_reason: input.finish_reason,
        content,
        tool_calls,
        citations: fields.get("citations").cloned(),
        usage: usage(fields)?,
    })
}

fn reply_tool_call(tool: &Json) -> Result<V1ToolCall, ReplyError> {
    let Json::Object(fields) = tool else {
        return Err(ReplyError::Field("tool_calls", "is not an object"));
    };
    let name = match fields.get("name") {
        None => Some(String::new()),
        Some(Json::Null) => None,
        Some(Json::Text(name)) => Some(name.clone()),
        Some(_) => return Err(ReplyError::Field("tool_calls.name", "is not a string")),
    };
    let generation_id = match fields.get("generation_id") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Text(id)) => id.clone(),
        Some(_) => return Err(ReplyError::Field("generation_id", "is not a string")),
    };
    // LiteLLM writes json.dumps(parameters) into the call with a default of {}
    let arguments = fields
        .get("parameters")
        .map_or_else(|| String::from("{}"), dumps);
    Ok(V1ToolCall {
        id: format!("call_{generation_id}"),
        kind: String::from("function"),
        function: V1Function { name, arguments },
    })
}

fn usage(fields: &Object) -> Result<CohereUsage, ReplyError> {
    let meta = object_field(fields.get("meta"), "meta")?;
    let billed_units = object_field(
        meta.and_then(|meta_fields| meta_fields.get("billed_units")),
        "meta.billed_units",
    )?;
    let prompt_tokens = count(
        billed_units.and_then(|units| units.get("input_tokens")),
        "input_tokens",
    )?;
    let completion_tokens = count(
        billed_units.and_then(|units| units.get("output_tokens")),
        "output_tokens",
    )?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(ReplyError::UsageOverflow)?;
    Ok(CohereUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

// a null key takes the field's default, so both absent and null count as no object here
const fn object_field<'value>(
    value: Option<&'value Json>,
    field: &'static str,
) -> Result<Option<&'value Object>, ReplyError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(_) => Err(ReplyError::Field(field, "is not an object")),
    }
}

fn count(value: Option<&Json>, field: &'static str) -> Result<i64, ReplyError> {
    let invalid = || ReplyError::Field(field, "is not an integer");
    match value {
        None | Some(Json::Null) => Ok(0),
        Some(Json::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(whole_number))
            .ok_or_else(invalid),
        Some(_) => Err(invalid()),
    }
}

// LiteLLM hands the counts to Usage, whose pydantic int field takes a whole float such as 9.0 as 9
fn whole_number(value: f64) -> Option<i64> {
    if value.fract().to_bits() != 0.0_f64.to_bits() {
        return None;
    }
    format!("{value:.0}").parse().ok()
}
