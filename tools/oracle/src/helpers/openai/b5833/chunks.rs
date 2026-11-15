use serde_json::{Value, json};
use uuid::Uuid;

use betterlite_openai::groq_chat::stream::chunk_parser;

use super::{Call, helper_error};
use crate::helpers::HelperError;

pub(super) fn chunk_values(calls: &[Call]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(calls.len());
    for call in calls {
        values.push(chunk_one(call)?);
    }
    Ok(values)
}

fn chunk_one(call: &Call) -> Result<Value, Box<HelperError>> {
    let chunk = call.chunk.clone().unwrap_or(Value::Null);
    // the Python adapter lets the same error out, so the case compares the helper error envelope
    let parsed = chunk_parser(&chunk, Uuid::nil, i64::default).map_err(|error| {
        helper_error(
            "ProviderError",
            Some(error.status_code()),
            error.to_string(),
        )
    })?;
    let delta = parsed.choices.first().map(|choice| &choice.delta);
    Ok(json!({
        "content": delta.and_then(|choice_delta| choice_delta.content.as_deref()),
        "reasoning_content": delta.and_then(|choice_delta| choice_delta.reasoning_content.as_deref()),
    }))
}
