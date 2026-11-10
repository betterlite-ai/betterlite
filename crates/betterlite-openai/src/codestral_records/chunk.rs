use std::str;

use betterlite_streaming::stream_chunk_shaping::extras::strip_sse_data_from_chunk;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codestral_records::error::TextCompletionCodestralError;
use crate::codestral_records::{MAX_RECORD_BYTES, RecordSource};
use crate::text_completion_policy::reply::mapped_finish_reason;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CodestralDelta {
    pub content: Option<String>,
    pub role: Option<String>,
    pub function_call: Option<Value>,
    pub tool_calls: Option<Value>,
    pub audio: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CodestralChunkUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodestralStreamChoice {
    pub index: i64,
    pub finish_reason: Option<String>,
    pub delta: CodestralDelta,
    pub logprobs: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodestralStreamChunk {
    pub id: String,
    pub created: i64,
    pub object: String,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub provider_specific_fields: Option<Value>,
    pub choices: Vec<CodestralStreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<CodestralChunkUsage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CodestralEmpty {
    pub text: String,
    pub is_finished: bool,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodestralContent {
    pub text: String,
    pub is_finished: bool,
    pub finish_reason: Option<String>,
    pub logprobs: Option<Value>,
    pub original_chunk: CodestralStreamChunk,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum CodestralChunk {
    Empty(CodestralEmpty),
    Content(Box<CodestralContent>),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct DeltaWire {
    content: Option<String>,
    role: Option<String>,
    function_call: Option<Value>,
    tool_calls: Option<Value>,
    audio: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct ChoiceWire {
    index: i64,
    delta: Option<DeltaWire>,
    finish_reason: Option<String>,
    logprobs: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct ChunkWire {
    id: Option<String>,
    created: Option<i64>,
    model: Option<String>,
    system_fingerprint: Option<String>,
    provider_specific_fields: Option<Value>,
    usage: Option<CodestralChunkUsage>,
    choices: Option<Vec<ChoiceWire>>,
}

pub fn decode_codestral_chunk(
    body: &[u8],
    source: &RecordSource,
) -> Result<CodestralChunk, TextCompletionCodestralError> {
    if body.len() > MAX_RECORD_BYTES {
        return Err(TextCompletionCodestralError::TooLarge {
            size: body.len(),
            limit: MAX_RECORD_BYTES,
        });
    }
    let line = str::from_utf8(body).map_err(TextCompletionCodestralError::BodyNotUtf8)?;
    let payload = strip_sse_data_from_chunk(Some(line)).unwrap_or(line).trim();
    if payload.is_empty() || payload == "[DONE]" {
        return Ok(CodestralChunk::Empty(CodestralEmpty::default()));
    }
    let chunk: Value = match serde_json::from_str(payload) {
        Ok(chunk) => chunk,
        Err(_invalid) => return Ok(CodestralChunk::Empty(CodestralEmpty::default())),
    };
    // ModelResponseStream(**chunk_data_dict) runs before the choices are looked at
    let wire: ChunkWire =
        serde_json::from_value(chunk).map_err(TextCompletionCodestralError::Stream)?;
    let Some(first) = wire.choices.as_deref().and_then(<[ChoiceWire]>::first) else {
        return Ok(CodestralChunk::Empty(CodestralEmpty::default()));
    };
    let text = first
        .delta
        .as_ref()
        .and_then(|delta| delta.content.clone())
        .unwrap_or_default();
    // A None check, not truthiness: an empty-string finish reason still finishes the answer
    let (is_finished, finish_reason, logprobs) = first
        .finish_reason
        .clone()
        .map_or((false, None, None), |reason| {
            (true, Some(reason), first.logprobs.clone())
        });
    Ok(CodestralChunk::Content(Box::new(CodestralContent {
        text,
        is_finished,
        finish_reason,
        logprobs,
        original_chunk: stream_chunk(&wire, source),
    })))
}

#[must_use]
fn stream_chunk(wire: &ChunkWire, source: &RecordSource) -> CodestralStreamChunk {
    let choices = wire.choices.iter().flatten().map(stream_choice).collect();
    CodestralStreamChunk {
        id: wire
            .id
            .clone()
            .unwrap_or_else(|| format!("chatcmpl-{}", source.id)),
        created: wire.created.unwrap_or(source.created),
        // ModelResponseStream.__init__ always sets the object, whatever the provider called it
        object: "chat.completion.chunk".to_owned(),
        model: wire.model.clone(),
        system_fingerprint: wire.system_fingerprint.clone(),
        provider_specific_fields: wire.provider_specific_fields.clone(),
        choices,
        usage: wire.usage.clone(),
    }
}

#[must_use]
fn stream_choice(choice: &ChoiceWire) -> CodestralStreamChoice {
    let finish_reason = choice
        .finish_reason
        .as_deref()
        .filter(|reason| !reason.is_empty())
        .map(|reason| mapped_finish_reason(reason).to_owned());
    CodestralStreamChoice {
        index: choice.index,
        finish_reason,
        delta: choice
            .delta
            .as_ref()
            .map_or_else(CodestralDelta::default, delta_from),
        logprobs: choice.logprobs.clone(),
    }
}

#[must_use]
fn delta_from(delta: &DeltaWire) -> CodestralDelta {
    CodestralDelta {
        content: delta.content.clone(),
        role: delta.role.clone(),
        function_call: delta.function_call.clone(),
        tool_calls: delta.tool_calls.clone(),
        audio: delta.audio.clone(),
    }
}
