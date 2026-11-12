use betterlite_openai::chat_choices::stream::{StreamError, chunk_parser};
use serde_json::{Value, json};
use uuid::Uuid;

use super::ChunkSteps;

pub(super) fn run(steps: &ChunkSteps) -> Value {
    let mut outputs = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let outcome = match chunk_parser(&step.chunk, Uuid::nil, || 7) {
            Ok(reply) => json!({"ok": true, "chunk": reply}),
            Err(error) => provider_outcome(&error),
        };
        outputs.push(outcome);
    }
    Value::Array(outputs)
}

fn provider_outcome(error: &StreamError) -> Value {
    match error {
        StreamError::Provider { status, .. } => json!({"ok": false, "status": status}),
        StreamError::Chunk(_) | StreamError::ToolCallPayload => {
            json!({"ok": false, "error": error.to_string()})
        }
    }
}
