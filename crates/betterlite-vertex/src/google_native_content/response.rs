use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_with::skip_serializing_none;

use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::gemini_response_parts::projection::Part;
use crate::gemini_usage::metadata::UsageMetadata;

use super::error::NativeContentError;

pub type NativeObject = IndexMap<String, Json>;

// the fallback GenerateContentResponse is a pydantic model with extra="allow", so unknown fields travel as sent
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateContentResponse {
    pub candidates: Option<Vec<CandidateRecord>>,
    pub prompt_feedback: Option<Json>,
    pub usage_metadata: Option<Json>,
    pub model_version: Option<String>,
    pub response_id: Option<String>,
    #[serde(flatten)]
    pub extension: NativeObject,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateRecord {
    pub content: Option<ContentRecord>,
    pub finish_reason: Option<String>,
    pub index: Option<i64>,
    pub citation_metadata: Option<Json>,
    #[serde(flatten)]
    pub extension: NativeObject,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ContentRecord {
    pub parts: Option<Vec<Json>>,
    pub role: Option<String>,
    #[serde(flatten)]
    pub extension: NativeObject,
}

impl ContentRecord {
    // the existing Gemini part record, decoded when a reader needs the typed view
    pub fn parts(&self) -> Result<Vec<Part>, NativeContentError> {
        self.parts.as_ref().map_or_else(
            || Ok(Vec::new()),
            |parts| parts.iter().map(decode_typed).collect(),
        )
    }
}

impl GenerateContentResponse {
    // UsageMetadata declares no passthrough, so the wire object stays and readers get the typed record
    pub fn usage(&self) -> Result<Option<UsageMetadata>, NativeContentError> {
        let Some(metadata) = self.usage_metadata.as_ref() else {
            return Ok(None);
        };
        decode_typed(metadata).map(Some)
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
    let Json::Object(mut fields) = serde_json::from_slice::<Json>(bytes)
        .map_err(|source| NativeContentError::Json { source })?
    else {
        return Err(NativeContentError::ResponseNotObject);
    };
    let candidates = match fields.shift_remove("candidates") {
        None | Some(Json::Null) => None,
        Some(Json::List(items)) => Some(
            items
                .into_iter()
                .map(candidate_record)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(NativeContentError::UnexpectedType {
                field: "candidates",
            });
        }
    };
    let prompt_feedback = take_object(&mut fields, "promptFeedback")?;
    let usage_metadata = take_object(&mut fields, "usageMetadata")?;
    let model_version = take_text(&mut fields, "modelVersion")?;
    let response_id = take_text(&mut fields, "responseId")?;
    Ok(GenerateContentResponse {
        candidates,
        prompt_feedback,
        usage_metadata,
        model_version,
        response_id,
        extension: fields,
    })
}

fn candidate_record(value: Json) -> Result<CandidateRecord, NativeContentError> {
    let Json::Object(mut fields) = value else {
        return Err(NativeContentError::UnexpectedType {
            field: "candidates",
        });
    };
    let content = match fields.shift_remove("content") {
        None | Some(Json::Null) => None,
        Some(Json::Object(content)) => Some(content_record(content)?),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(NativeContentError::UnexpectedType { field: "content" });
        }
    };
    let finish_reason = take_text(&mut fields, "finishReason")?;
    let index = take_index(&mut fields)?;
    // LiteLLM rewrites only an object citationMetadata and leaves any other value as it arrived
    let mut citation_metadata = fields
        .shift_remove("citationMetadata")
        .filter(|metadata| !matches!(metadata, Json::Null));
    convert_citation_sources(&mut citation_metadata);
    Ok(CandidateRecord {
        content,
        finish_reason,
        index,
        citation_metadata,
        extension: fields,
    })
}

fn content_record(mut fields: NativeObject) -> Result<ContentRecord, NativeContentError> {
    let parts = match fields.shift_remove("parts") {
        None | Some(Json::Null) => None,
        Some(Json::List(items)) => Some(items),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(NativeContentError::UnexpectedType { field: "parts" });
        }
    };
    let role = take_text(&mut fields, "role")?;
    Ok(ContentRecord {
        parts,
        role,
        extension: fields,
    })
}

// GoogleGenAIConfig.convert_citation_sources_to_citations replaces citations with the sources it pops
fn convert_citation_sources(metadata: &mut Option<Json>) {
    let Some(Json::Object(fields)) = metadata else {
        return;
    };
    let Some(sources) = fields.shift_remove("citationSources") else {
        return;
    };
    fields.insert("citations".to_owned(), sources);
}

fn take_text(
    fields: &mut NativeObject,
    name: &'static str,
) -> Result<Option<String>, NativeContentError> {
    match fields.shift_remove(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(NativeContentError::UnexpectedType { field: name })
        }
    }
}

fn take_object(
    fields: &mut NativeObject,
    name: &'static str,
) -> Result<Option<Json>, NativeContentError> {
    match fields.shift_remove(name) {
        None | Some(Json::Null) => Ok(None),
        Some(object @ Json::Object(_)) => Ok(Some(object)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(NativeContentError::UnexpectedType { field: name })
        }
    }
}

fn take_index(fields: &mut NativeObject) -> Result<Option<i64>, NativeContentError> {
    match fields.shift_remove("index") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or_else(|| NativeContentError::UnexpectedType { field: "index" }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(NativeContentError::UnexpectedType { field: "index" })
        }
    }
}

pub struct RawResponse<'raw> {
    pub status: u16,
    pub headers: &'raw IndexMap<String, String>,
    pub body: &'raw [u8],
}

// transform_response: the raw reply stays beside the record, as LiteLLM keeps it for logging
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
