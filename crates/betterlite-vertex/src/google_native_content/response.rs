use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::gemini_candidates::candidate::Candidate;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::gemini_usage::metadata::UsageMetadata;

use super::error::NativeContentError;

// the fallback response type declares no fields and accepts any, so the record keeps the reply exactly as sent
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct GenerateContentResponse {
    pub wire: Object,
}

impl GenerateContentResponse {
    // the existing Gemini candidate record, parts and candidate extension included
    pub fn candidates(&self) -> Result<Vec<Candidate>, NativeContentError> {
        match self.wire.get("candidates") {
            None | Some(Json::Null) => Ok(Vec::new()),
            Some(candidates) => decode_typed(candidates),
        }
    }

    // UsageMetadata declares no passthrough, so readers get the typed record and the wire keeps the object
    pub fn usage(&self) -> Result<Option<UsageMetadata>, NativeContentError> {
        match self.wire.get("usageMetadata") {
            None | Some(Json::Null) => Ok(None),
            Some(metadata) => decode_typed(metadata).map(Some),
        }
    }
}

fn decode_typed<T>(value: &Json) -> Result<T, NativeContentError>
where
    T: DeserializeOwned,
{
    let encoded =
        serde_json::to_value(value).map_err(|source| NativeContentError::Json { source })?;
    serde_json::from_value(encoded).map_err(|source| NativeContentError::Json { source })
}

// the transport caps a reply before the transform sees it, so a body past that cap is already an error
pub fn decode_google_native_content(
    bytes: &[u8],
) -> Result<GenerateContentResponse, NativeContentError> {
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(NativeContentError::ResponseTooLarge {
            size: bytes.len(),
            limit: MAX_RESPONSE_BYTES,
        });
    }
    let Json::Object(mut wire) = serde_json::from_slice::<Json>(bytes)
        .map_err(|source| NativeContentError::Json { source })?
    else {
        return Err(NativeContentError::ResponseNotObject);
    };
    convert_citation_sources(&mut wire);
    Ok(GenerateContentResponse { wire })
}

// GoogleGenAIConfig.convert_citation_sources_to_citations rewrites a candidate's metadata in place
fn convert_citation_sources(wire: &mut Object) {
    let Some(Json::List(candidates)) = wire.get_mut("candidates") else {
        return;
    };
    for entry in candidates {
        let Json::Object(candidate) = entry else {
            continue;
        };
        let Some(Json::Object(metadata)) = candidate.get_mut("citationMetadata") else {
            continue;
        };
        let Some(sources) = metadata.shift_remove("citationSources") else {
            continue;
        };
        metadata.insert("citations".to_owned(), sources);
    }
}

pub struct RawResponse<'raw> {
    pub status: u16,
    pub headers: &'raw IndexMap<String, String>,
    pub body: &'raw [u8],
}

// transform_generate_content_response: the raw reply stays beside the record, as LiteLLM keeps it for logging
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeResponse {
    pub response: GenerateContentResponse,
    pub status: u16,
    pub headers: IndexMap<String, String>,
}

pub fn transform_response(raw: &RawResponse<'_>) -> Result<NativeResponse, NativeContentError> {
    let response =
        decode_google_native_content(raw.body).map_err(|source| NativeContentError::Response {
            status: raw.status,
            headers: raw.headers.clone(),
            source: Box::new(source),
        })?;
    Ok(NativeResponse {
        response,
        status: raw.status,
        headers: raw.headers.clone(),
    })
}
