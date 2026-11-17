use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use indexmap::IndexMap;

use crate::api::chat_response_stream_items::snapshot::snapshot_chunk;
use crate::api::chat_response_stream_tools::fields::{list, object};

use super::error::ChatResponseStreamError;

// One chunk as ModelResponseStream dumps it, with its private _hidden_params folded back in before the snapshot
pub fn wrapper_view(chunk: &StreamChunk) -> Result<Json, ChatResponseStreamError> {
    let encoded = serde_json::to_vec(chunk)?;
    let fields: IndexMap<String, Json> = serde_json::from_slice(&encoded)?;
    Ok(snapshot_chunk(&fold_hidden_params(fields)))
}

// ModelResponseStream keeps hidden parameters under a private name, and the assembler reads that name
#[must_use]
pub fn fold_hidden_params(mut fields: IndexMap<String, Json>) -> IndexMap<String, Json> {
    if let Some(hidden) = fields.shift_remove("hidden_params") {
        fields.insert("_hidden_params".to_owned(), hidden);
    }
    fields
}

// LiteLLM reads the first choice only, whatever else the provider put in the chunk
#[must_use]
pub fn first_choice(chunk: &Json) -> Option<&IndexMap<String, Json>> {
    object(chunk)
        .and_then(|fields| fields.get("choices"))
        .and_then(list)
        .and_then(|choices| choices.first())
        .and_then(object)
}

#[must_use]
pub fn delta(chunk: &Json) -> Option<&IndexMap<String, Json>> {
    first_choice(chunk)
        .and_then(|choice| choice.get("delta"))
        .and_then(object)
}
