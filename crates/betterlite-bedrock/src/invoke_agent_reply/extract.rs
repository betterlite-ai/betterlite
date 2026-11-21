use std::str::from_utf8;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::converse_events::wire::non_empty;
use crate::invoke_agent_reply::events::AgentEvent;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageInfo {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub model: Option<String>,
}

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("event field {field} must be a JSON object")]
    NotObject { field: &'static str },
    #[error("event field {field} must be an integer")]
    NotInteger { field: &'static str },
    #[error("event field {field} must be a string")]
    NotText { field: &'static str },
    #[error("token usage total overflows")]
    Overflow,
}

// _extract_response_content: every chunk's re-encoded base64 bytes decoded back to text and joined in arrival order.
pub fn response_content(events: &[AgentEvent]) -> Result<String, ExtractError> {
    let mut content = String::new();
    for event in events {
        if event.headers.event_type != "chunk" || !non_empty(&event.payload) {
            continue;
        }
        let fields = object(&event.payload).ok_or(ExtractError::NotObject {
            field: "chunk payload",
        })?;
        let Some(bytes) = fields.get("bytes") else {
            continue;
        };
        if !non_empty(bytes) {
            continue;
        }
        append_chunk(bytes, &mut content);
    }
    Ok(content)
}

// _extract_usage_info: the pre-processing token sums and the first orchestration model any trace names.
pub fn usage_info(events: &[AgentEvent]) -> Result<UsageInfo, ExtractError> {
    let mut usage = UsageInfo::default();
    for event in events {
        if event.headers.event_type != "trace" || !non_empty(&event.payload) {
            continue;
        }
        let payload = object(&event.payload).ok_or(ExtractError::NotObject {
            field: "trace payload",
        })?;
        let Some(trace_value) = payload.get("trace").filter(|value| non_empty(value)) else {
            continue;
        };
        let trace = object(trace_value).ok_or(ExtractError::NotObject { field: "trace" })?;
        preprocessing_usage(trace, &mut usage)?;
        if usage.model.is_none() {
            usage.model = orchestration_model(trace)?;
        }
    }
    Ok(usage)
}

fn append_chunk(bytes: &Json, content: &mut String) {
    let Json::Text(encoded) = bytes else {
        tracing::warn!("Failed to decode chunk content: chunk bytes are not a string");
        return;
    };
    let decoded = match STANDARD.decode(encoded) {
        Ok(decoded) => decoded,
        Err(error) => {
            tracing::warn!("Failed to decode chunk content: {error}");
            return;
        }
    };
    match from_utf8(&decoded) {
        Ok(text) => content.push_str(text),
        Err(error) => tracing::warn!("Failed to decode chunk content: {error}"),
    }
}

fn preprocessing_usage(
    trace: &IndexMap<String, Json>,
    usage: &mut UsageInfo,
) -> Result<(), ExtractError> {
    let Some(pre) = usable_object(trace.get("preProcessingTrace"), "preProcessingTrace")? else {
        return Ok(());
    };
    let Some(output) = usable_object(pre.get("modelInvocationOutput"), "modelInvocationOutput")?
    else {
        return Ok(());
    };
    let Some(metadata) = usable_object(output.get("metadata"), "metadata")? else {
        return Ok(());
    };
    let Some(counts) = usable_object(metadata.get("usage"), "usage")? else {
        return Ok(());
    };
    let input_tokens = count(counts.get("inputTokens"), "inputTokens")?;
    let output_tokens = count(counts.get("outputTokens"), "outputTokens")?;
    usage.input_tokens = usage
        .input_tokens
        .checked_add(input_tokens)
        .ok_or(ExtractError::Overflow)?;
    usage.output_tokens = usage
        .output_tokens
        .checked_add(output_tokens)
        .ok_or(ExtractError::Overflow)?;
    Ok(())
}

fn orchestration_model(trace: &IndexMap<String, Json>) -> Result<Option<String>, ExtractError> {
    let Some(orchestration) = usable_object(trace.get("orchestrationTrace"), "orchestrationTrace")?
    else {
        return Ok(None);
    };
    let Some(invocation) = usable_object(
        orchestration.get("modelInvocationInput"),
        "modelInvocationInput",
    )?
    else {
        return Ok(None);
    };
    match invocation.get("foundationModel") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(model)) => Ok(Some(model.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ExtractError::NotText {
                field: "foundationModel",
            })
        }
    }
}

// LiteLLM tests every intermediate with bool(), so an absent, null or empty one ends the walk.
fn usable_object<'value>(
    value: Option<&'value Json>,
    field: &'static str,
) -> Result<Option<&'value IndexMap<String, Json>>, ExtractError> {
    value
        .filter(|value| non_empty(value))
        .map(|value| object(value).ok_or(ExtractError::NotObject { field }))
        .transpose()
}

const fn object<'value>(value: &'value Json) -> Option<&'value IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn count(value: Option<&Json>, field: &'static str) -> Result<i64, ExtractError> {
    match value {
        None => Ok(0),
        Some(Json::Number(number)) => number.as_i64().ok_or(ExtractError::NotInteger { field }),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ExtractError::NotInteger { field })
        }
    }
}
