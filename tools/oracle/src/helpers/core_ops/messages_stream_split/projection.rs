use betterlite_streaming::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChoice, StreamChunk, Usage,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct ChunkView {
    id: String,
    object: String,
    created: i64,
    model: String,
    system_fingerprint: Option<String>,
    provider_specific_fields: Option<JsonObject>,
    choices: Vec<ChoiceView>,
    usage: Option<UsageView>,
    hidden_params: JsonObject,
}

#[derive(Debug, Serialize)]
pub(super) struct ChoiceView {
    index: i64,
    finish_reason: Option<String>,
    delta: DeltaView,
}

#[derive(Debug, Serialize)]
pub(super) struct DeltaView {
    role: Option<String>,
    content: Option<String>,
    reasoning_content: Option<String>,
    thinking_blocks: Option<JsonValue>,
    tool_calls: Option<JsonValue>,
    function_call: Option<JsonValue>,
    audio: Option<JsonValue>,
    images: Option<JsonValue>,
    annotations: Option<JsonValue>,
    refusal: Option<JsonValue>,
    provider_specific_fields: Option<JsonObject>,
}

#[derive(Debug, Serialize)]
pub(super) struct UsageView {
    #[serde(rename = "prompt_tokens")]
    prompt: i64,
    #[serde(rename = "completion_tokens")]
    completion: i64,
    #[serde(rename = "total_tokens")]
    total: i64,
}

#[derive(Debug, Serialize)]
pub(super) struct StatusView {
    pub(super) status: u16,
    pub(super) message: String,
}

const fn usage_view(usage: &Usage) -> UsageView {
    UsageView {
        prompt: usage.prompt_tokens,
        completion: usage.completion_tokens,
        total: usage.total_tokens,
    }
}

fn choice_view(choice: &StreamChoice) -> ChoiceView {
    ChoiceView {
        index: choice.index,
        finish_reason: choice.finish_reason.clone(),
        delta: DeltaView {
            role: choice.delta.role.clone(),
            content: choice.delta.content.clone(),
            reasoning_content: choice.delta.reasoning_content.clone(),
            thinking_blocks: choice.delta.thinking_blocks.clone(),
            tool_calls: choice.delta.tool_calls.clone(),
            function_call: choice.delta.function_call.clone(),
            audio: choice.delta.audio.clone(),
            images: choice.delta.images.clone(),
            annotations: choice.delta.annotations.clone(),
            refusal: choice.delta.extra.get("refusal").cloned(),
            provider_specific_fields: choice.delta.provider_specific_fields.clone(),
        },
    }
}

pub(super) fn chunk_view(chunk: &StreamChunk) -> ChunkView {
    ChunkView {
        id: chunk.id.clone(),
        object: chunk.object.clone(),
        created: chunk.created,
        model: chunk.model.clone(),
        system_fingerprint: chunk.system_fingerprint.clone(),
        provider_specific_fields: chunk.provider_specific_fields.clone(),
        choices: chunk.choices.iter().map(choice_view).collect(),
        usage: chunk.usage.as_ref().map(usage_view),
        hidden_params: chunk.hidden_params.clone(),
    }
}
