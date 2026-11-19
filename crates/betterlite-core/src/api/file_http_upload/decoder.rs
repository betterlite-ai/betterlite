use betterlite_models::decisions::Json;
use betterlite_openai::api::file_type_records::UploadUrlLocation;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary};

// Gemini and the Vertex resumable protocol answer the upload URL in this header, under any casing
const UPLOAD_URL_HEADER: &str = "x-goog-upload-url";

// What _extract_upload_url_from_response returns: the URL, and the parsed body only when it held one
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocatedUpload {
    pub url: Option<String>,
    pub data: Option<IndexMap<String, Json>>,
}

#[must_use]
pub fn upload_url_from_headers(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(UPLOAD_URL_HEADER)?;
    let url = value.to_str().ok()?;
    // an empty header is as absent as a missing one: LiteLLM tests `if not upload_url`
    (!url.is_empty()).then(|| url.to_owned())
}

// _extract_upload_url_from_response's body branch, bounded by B1087: a body that is not a JSON object,
// or whose key is missing, null, empty or not a string, names no URL and keeps no response data
pub fn decode_file_upload_location(
    response_bytes: &[u8],
    location: UploadUrlLocation,
    key: &str,
    limits: ApiLimits,
) -> Result<LocatedUpload, ApiLimitError> {
    if location == UploadUrlLocation::Headers {
        // the header branch never reads the body, so an oversized one costs nothing
        return Ok(LocatedUpload::default());
    }
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: response_bytes.len(),
    }))?;
    let Ok(Json::Object(fields)) = serde_json::from_slice::<Json>(response_bytes) else {
        return Ok(LocatedUpload::default());
    };
    let Some(url) = located_url(fields.get(key)) else {
        return Ok(LocatedUpload::default());
    };
    Ok(LocatedUpload {
        url: Some(url),
        data: Some(fields),
    })
}

fn located_url(value: Option<&Json>) -> Option<String> {
    match value? {
        Json::Text(text) if !text.is_empty() => Some(text.clone()),
        Json::Text(_)
        | Json::Bool(_)
        | Json::Null
        | Json::Number(_)
        | Json::List(_)
        | Json::Object(_) => None,
    }
}
