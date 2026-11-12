use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;

use crate::gemini_candidates::candidate::{Candidate, PromptFeedback};
use crate::gemini_response::error::GenerateContentError;
use crate::gemini_usage::metadata::UsageMetadata;

// the pooled transport caps a reply at 16 MiB; a decoder handed more than that refuses it
pub const MAX_RESPONSE_BYTES: usize = 0x0100_0000;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawGenerateContent {
    candidates: Option<Vec<Candidate>>,
    prompt_feedback: Option<PromptFeedback>,
    usage_metadata: Option<Box<RawValue>>,
    response_id: Option<String>,
    model_version: Option<Json>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageTrafficType {
    traffic_type: Option<String>,
}

#[derive(Debug)]
struct DecodedUsage {
    metadata: UsageMetadata,
    traffic_type: Option<String>,
}

#[derive(Debug)]
pub struct GenerateContentResponse {
    pub candidates: Option<Vec<Candidate>>,
    pub prompt_feedback: Option<PromptFeedback>,
    pub usage_metadata: Option<UsageMetadata>,
    pub traffic_type: Option<String>,
    pub response_id: Option<String>,
    pub model_version: Option<Json>,
}

pub fn decode_generate_content(
    bytes: &[u8],
) -> Result<GenerateContentResponse, GenerateContentError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(GenerateContentError::TooLarge { size: bytes.len() });
    }
    let raw: RawGenerateContent = decode_json(bytes)?;
    let usage = raw
        .usage_metadata
        .as_deref()
        .map(decode_usage)
        .transpose()?;
    Ok(GenerateContentResponse {
        candidates: raw.candidates,
        prompt_feedback: raw.prompt_feedback,
        traffic_type: usage
            .as_ref()
            .and_then(|decoded| decoded.traffic_type.clone()),
        usage_metadata: usage.map(|decoded| decoded.metadata),
        response_id: raw.response_id,
        model_version: raw.model_version,
    })
}

// UsageMetadata declares no trafficType, so the runtime extra comes from a second typed parse
fn decode_usage(raw: &RawValue) -> Result<DecodedUsage, GenerateContentError> {
    let metadata: UsageMetadata = decode_json(raw.get().as_bytes())?;
    let traffic: UsageTrafficType = decode_json(raw.get().as_bytes())?;
    Ok(DecodedUsage {
        metadata,
        traffic_type: traffic
            .traffic_type
            .filter(|traffic_type| !traffic_type.is_empty()),
    })
}

fn decode_json<T>(bytes: &[u8]) -> Result<T, GenerateContentError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        GenerateContentError::Json {
            path: error.path().to_string(),
            source: error.into_inner(),
        }
    })?;
    deserializer
        .end()
        .map_err(|source| GenerateContentError::Json {
            path: "the document root".to_owned(),
            source,
        })?;
    Ok(value)
}
