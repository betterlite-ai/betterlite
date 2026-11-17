use betterlite_streaming::stream_chunk_shaping::records::JsonValue;
use betterlite_streaming::streamed_response_assembly::records::{Choice, Chunk, Delta, TextField};
use serde::Deserialize;

use crate::cisco_conversation::Fields;

const CHUNK_OBJECT: &str = "chat.completion.chunk";

#[derive(Deserialize, Default)]
#[serde(default)]
struct RebuiltResponse {
    id: String,
    created: i64,
    model: String,
    choices: Vec<RebuiltChoice>,
    usage: Option<JsonValue>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RebuiltChoice {
    index: i64,
    finish_reason: String,
    message: Delta,
}

pub(super) fn mock_response_chunks(response: &Fields) -> Result<Vec<Chunk>, serde_json::Error> {
    let text = serde_json::to_string(response)?;
    let rebuilt: RebuiltResponse = serde_json::from_str(&text)?;
    let chunk = Chunk {
        hidden_params: None,
        id: Some(rebuilt.id),
        object: Some(CHUNK_OBJECT.to_owned()),
        created: Some(rebuilt.created),
        model: Some(rebuilt.model),
        system_fingerprint: None,
        service_tier: None,
        usage: rebuilt.usage,
        choices: rebuilt.choices.into_iter().map(rebuilt_choice).collect(),
    };
    Ok(vec![chunk])
}

fn rebuilt_choice(item: RebuiltChoice) -> Choice {
    Choice {
        index: u32::try_from(item.index).ok(),
        finish_reason: Some(item.finish_reason),
        logprobs: None,
        delta: Some(item.message),
        text: TextField::Absent,
    }
}
