use serde_json::{Value, json};
use uuid::Uuid;

use betterlite_openai::groq_chat::stream::chunk_parser;

use super::Call;

pub(super) fn chunk_values(calls: &[Call]) -> Vec<Value> {
    calls.iter().map(chunk_one).collect()
}

// an error chunk becomes an item, so one case compares the parsed chunks beside the error
fn chunk_one(call: &Call) -> Value {
    let chunk = call.chunk.clone().unwrap_or(Value::Null);
    let parsed = match chunk_parser(&chunk, Uuid::nil, i64::default) {
        Ok(parsed) => parsed,
        Err(error) => return json!({"error": {"status": error.status_code()}}),
    };
    let delta = parsed.choices.first().map(|choice| &choice.delta);
    json!({
        "content": delta.and_then(|choice_delta| choice_delta.content.as_deref()),
        "reasoning_content": delta.and_then(|choice_delta| choice_delta.reasoning_content.as_deref()),
    })
}
