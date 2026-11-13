use betterlite_models::decisions::Json;

use super::StreamEventConverter;
use super::chunk::Branch;
use super::dispatch::{encode, event_fields, has_value};
use super::error::{AnthropicError, ShapeError, StreamEventError};
use crate::stream_state::code::build_code_interpreter_results;
use crate::stream_state::message::{handle_message_delta, handle_usage};
use crate::stream_state::wire::JsonFields;
use crate::usage::ReasoningTokenCounter;

pub(crate) fn message_delta<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    chunk: &Json,
    branch: &mut Branch,
) -> Result<(), StreamEventError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let projection = handle_message_delta(&mut converter.state, chunk, &converter.counter)?;
    branch.finish_reason = Some(projection.finish_reason);
    branch.usage = projection.usage;
    let Some(container) = projection.container.filter(has_value) else {
        return Ok(());
    };
    branch
        .provider_specific_fields
        .insert("container".to_owned(), container.clone());
    let container_id = match &container {
        Json::Object(fields) => match fields.get("id") {
            Some(Json::Text(text)) if !text.is_empty() => Some(text.clone()),
            _ => None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    };
    let Some(stored_id) = container_id else {
        return Ok(());
    };
    if converter.state.tool_results.is_empty() {
        return Ok(());
    }
    converter.state.container_id = Some(stored_id);
    let results = build_code_interpreter_results(&converter.state).map_err(ShapeError::Event)?;
    branch
        .provider_specific_fields
        .insert("code_interpreter_results".to_owned(), encode(&results)?);
    Ok(())
}

pub(crate) fn message_start<Counter>(
    converter: &mut StreamEventConverter<Counter>,
    chunk: &Json,
    branch: &mut Branch,
) -> Result<(), StreamEventError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let fields = event_fields(chunk)?;
    let message = event_field(fields, "message")?;
    if let Some(model) = message.get("model") {
        converter.state.served_model = optional_text(model, "message.model")?;
    }
    if let Some(usage) = message.get("usage") {
        branch.usage = Some(handle_usage(
            &mut converter.state,
            usage,
            &converter.counter,
        )?);
    }
    Ok(())
}

fn event_field<'value>(
    fields: &'value JsonFields,
    name: &'static str,
) -> Result<&'value JsonFields, ShapeError> {
    match fields.get(name) {
        Some(Json::Object(value)) => Ok(value),
        Some(_) => Err(ShapeError::FieldType { field: name }),
        None => Err(ShapeError::Missing { field: name }),
    }
}

fn optional_text(value: &Json, field: &'static str) -> Result<Option<String>, ShapeError> {
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(ShapeError::FieldType { field })
        }
    }
}

pub(crate) fn native_error(chunk: &Json, fields: &JsonFields) -> AnthropicError {
    let message = match fields.get("error") {
        Some(Json::Object(error)) => match error.get("message") {
            Some(Json::Text(message)) if !message.is_empty() => Some(message.clone()),
            _ => None,
        },
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    AnthropicError {
        message: message.unwrap_or_else(|| serde_json::to_string(chunk).unwrap_or_default()),
        status_code: 500,
    }
}
