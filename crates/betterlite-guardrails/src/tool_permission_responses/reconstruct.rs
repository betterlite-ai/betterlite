use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_message_scope::Object;
use crate::tool_permission::types::{mapping_value, object_list};

const CHAT_COMPLETION_CHUNK: &str = "chat.completion.chunk";

// LiteLLM's MockResponseIterator: one chunk whose delta is the assembled message, with its finish_reason kept.
#[must_use]
pub(crate) fn reconstruct_chunk(response: &Object) -> Object {
    let mut chunk = Object::new();
    for key in ["id", "created", "model"] {
        if let Some(value) = response.get(key) {
            chunk.insert(key.to_owned(), value.clone());
        }
    }
    chunk.insert(
        "object".to_owned(),
        GuardrailValue::Text(CHAT_COMPLETION_CHUNK.to_owned()),
    );
    let choices = response
        .get("choices")
        .and_then(object_list)
        .map_or_else(Vec::new, |choices| {
            choices.iter().map(stream_choice).collect()
        });
    chunk.insert("choices".to_owned(), GuardrailValue::Array(choices));
    if let Some(usage) = response.get("usage") {
        chunk.insert("usage".to_owned(), usage.clone());
    }
    chunk
}

// Delta carries every message field, which is how convert_model_response_to_streaming renames message to delta.
fn stream_choice(choice: &GuardrailValue) -> GuardrailValue {
    let index = mapping_value(choice, "index")
        .cloned()
        .unwrap_or(GuardrailValue::Null);
    let delta = mapping_value(choice, "message")
        .cloned()
        .unwrap_or(GuardrailValue::Null);
    let finish_reason = mapping_value(choice, "finish_reason")
        .cloned()
        .unwrap_or(GuardrailValue::Null);
    let mut fields = Object::new();
    fields.insert("index".to_owned(), index);
    fields.insert("delta".to_owned(), delta);
    fields.insert("finish_reason".to_owned(), finish_reason);
    if let Some(logprobs) =
        mapping_value(choice, "logprobs").filter(|value| !matches!(value, GuardrailValue::Null))
    {
        fields.insert("logprobs".to_owned(), logprobs.clone());
    }
    GuardrailValue::Object(fields)
}

mod tests;
