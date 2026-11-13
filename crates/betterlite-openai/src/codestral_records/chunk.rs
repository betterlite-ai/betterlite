use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use betterlite_streaming::stream_chunk_shaping::records::{
    ChunkArguments, JsonObject, JsonValue, StreamChoice, StreamChunk,
};
use serde::Serialize;
use serde_json::Value;

use crate::codestral_records::error::TextCompletionCodestralError;
use crate::codestral_records::{RecordSource, record_text};
use crate::text_completion_policy::reply::mapped_finish_reason;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CodestralChunk {
    pub text: String,
    pub is_finished: bool,
    pub finish_reason: Option<String>,
    pub logprobs: Option<JsonValue>,
    #[serde(skip)]
    pub original_chunk: Option<StreamChunk>,
}

pub fn decode_codestral_chunk(
    body: &[u8],
    source: &RecordSource,
) -> Result<CodestralChunk, TextCompletionCodestralError> {
    let raw = record_text(body)?;
    let payload = strip_sse_data_from_chunk(Some(&raw)).unwrap_or(&raw).trim();
    let Ok(chunk) = serde_json::from_str::<Value>(payload) else {
        return Ok(CodestralChunk::default());
    };
    // ModelResponseStream(**chunk_data_dict) runs before the choices are looked at
    let args: ChunkArguments =
        serde_json::from_value(chunk).map_err(TextCompletionCodestralError::Stream)?;
    let Some(first) = args.choices.as_deref().and_then(<[StreamChoice]>::first) else {
        return Ok(CodestralChunk::default());
    };
    let text = first.delta.content.clone().unwrap_or_default();
    // A None check, not truthiness: an empty-string finish reason still finishes the answer
    let finish_reason = first.finish_reason.clone();
    let logprobs = first.logprobs.clone().filter(|_| finish_reason.is_some());
    Ok(CodestralChunk {
        text,
        is_finished: finish_reason.is_some(),
        finish_reason,
        logprobs,
        original_chunk: Some(original_chunk(args, source)),
    })
}

fn original_chunk(mut args: ChunkArguments, source: &RecordSource) -> StreamChunk {
    let mut choices = args.choices.take().unwrap_or_default();
    for choice in &mut choices {
        choice.finish_reason = choice
            .finish_reason
            .take()
            .filter(|reason| !reason.is_empty())
            .map(|reason| mapped_finish_reason(&reason).to_owned());
    }
    let model = args
        .extra
        .remove("model")
        .and_then(|model| model.as_str().map(str::to_owned))
        .unwrap_or_default();
    args.extra.remove("object");
    StreamChunk {
        id: args.id.unwrap_or_else(|| format!("chatcmpl-{}", source.id)),
        created: args.created.unwrap_or(source.created),
        model,
        system_fingerprint: args.system_fingerprint,
        choices,
        usage: args.usage,
        provider_specific_fields: args.provider_specific_fields,
        extra: args.extra,
        object: "chat.completion.chunk".to_owned(),
        hidden_params: JsonObject::new(),
    }
}
