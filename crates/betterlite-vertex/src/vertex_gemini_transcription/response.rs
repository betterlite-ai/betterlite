use betterlite_http::proxy::UpstreamResponse;
use serde::Serialize;
use serde_json::Deserializer;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::vertex_gemini_transcription::error::VertexGeminiTranscriptionError;
use crate::vertex_gemini_transcription::types::{ResponseBody, UsageMetadata};
use crate::vertex_recognition::response::TranscriptionTask;

pub const AUDIO_MODALITY: &str = "AUDIO";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageKind {
    Tokens,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InputTokenDetails {
    pub audio_tokens: i64,
    pub text_tokens: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TranscriptionUsage {
    #[serde(rename = "type")]
    pub kind: UsageKind,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub input_token_details: InputTokenDetails,
}

// LiteLLM keeps the provider envelope in hidden params only for Speech-to-Text, so this projection drops it
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GeminiTranscriptionResponse {
    pub text: String,
    pub task: TranscriptionTask,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TranscriptionUsage>,
}

// ast-grep-ignore: no-long-names
pub fn decode_vertex_gemini_transcription(
    reply: &UpstreamResponse,
) -> Result<GeminiTranscriptionResponse, VertexGeminiTranscriptionError> {
    if reply.body.len() > MAX_RESPONSE_BYTES {
        return Err(VertexGeminiTranscriptionError::TooLarge {
            size: reply.body.len(),
        });
    }
    let mut decoder = Deserializer::from_slice(&reply.body);
    let parsed: ResponseBody = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|error| decode_error(reply.status, error))?;
    decoder
        .end()
        .map_err(|source| VertexGeminiTranscriptionError::NotJson {
            status: reply.status,
            source,
        })?;
    project(&parsed)
}

// a syntax failure is the source's non-JSON reply, while a typed field that fails is pydantic's ValidationError
fn decode_error(
    status: u16,
    error: serde_path_to_error::Error<serde_json::Error>,
) -> VertexGeminiTranscriptionError {
    let path = error.path().to_string();
    let source = error.into_inner();
    if source.is_data() {
        return VertexGeminiTranscriptionError::Shape { path, source };
    }
    VertexGeminiTranscriptionError::NotJson { status, source }
}

fn project(
    parsed: &ResponseBody,
) -> Result<GeminiTranscriptionResponse, VertexGeminiTranscriptionError> {
    let texts: Vec<&str> = parsed
        .candidates
        .iter()
        .filter_map(|candidate| candidate.content.as_ref())
        .flat_map(|content| content.parts.iter())
        .filter_map(|part| part.text.as_deref())
        .filter(|text| !text.is_empty())
        .collect();
    Ok(GeminiTranscriptionResponse {
        text: texts.join(" "),
        task: TranscriptionTask::Transcribe,
        usage: parsed
            .usage_metadata
            .as_ref()
            .map(usage_from_metadata)
            .transpose()?,
    })
}

fn usage_from_metadata(
    metadata: &UsageMetadata,
) -> Result<TranscriptionUsage, VertexGeminiTranscriptionError> {
    let audio_tokens = metadata
        .prompt_tokens_details
        .iter()
        .filter(|detail| detail.modality.as_deref() == Some(AUDIO_MODALITY))
        .try_fold(0_i64, |total, detail| total.checked_add(detail.token_count))
        .ok_or(VertexGeminiTranscriptionError::CountOverflow)?;
    // LiteLLM subtracts without clamping, so a prompt count below the audio count stays negative
    let text_tokens = metadata
        .prompt_token_count
        .checked_sub(audio_tokens)
        .ok_or(VertexGeminiTranscriptionError::CountOverflow)?;
    Ok(TranscriptionUsage {
        kind: UsageKind::Tokens,
        input_tokens: metadata.prompt_token_count,
        output_tokens: metadata.candidates_token_count,
        total_tokens: metadata.total_token_count,
        input_token_details: InputTokenDetails {
            audio_tokens,
            text_tokens,
        },
    })
}
