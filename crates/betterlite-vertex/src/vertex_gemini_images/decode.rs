use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json as ResponseJson;
use serde::Deserialize;
use serde::de::{DeserializeOwned, Error as _};
use serde_json::Error as JsonError;
use serde_json::error::Category;
use serde_json::value::RawValue;

use super::error::GeminiImageError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

// transform_image_generation_response reads the reply as a dict; only the fields it consumes are declared
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleImageResponse {
    pub candidates: Option<Vec<GoogleImageCandidate>>,
    pub usage_metadata: Option<Box<RawValue>>,
}

// the image usage's declared shape, with absent and null counts both taking zero
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageUsageMetadata {
    pub prompt_token_count: Option<i64>,
    pub candidates_token_count: Option<i64>,
    pub total_token_count: Option<i64>,
    pub prompt_tokens_details: Option<Vec<ImageTokenCountDetail>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageTokenCountDetail {
    pub modality: String,
    pub token_count: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleImageCandidate {
    pub content: Option<GoogleImageContent>,
    // the count only needs the searches, so the metadata stays as the provider wrote it
    pub grounding_metadata: Option<ResponseJson>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleImageContent {
    pub parts: Vec<GoogleImagePart>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleImagePart {
    pub inline_data: Option<GoogleImageInlineData>,
    pub thought_signature: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleImageInlineData {
    // mimeType never filters an image: a present data string is appended as it came
    pub data: Option<String>,
}

pub fn decode_google_image(
    reply: &UpstreamResponse,
) -> Result<GoogleImageResponse, GeminiImageError> {
    if reply.body.len() > MAX_RESPONSE_BYTES {
        return Err(GeminiImageError::TooLarge {
            size: reply.body.len(),
        });
    }
    require_object(&reply.body)?;
    decode_json(reply, &reply.body)
}

// {} is falsy in LiteLLM, while an object holding only nulls stays a present usage with zero counts
pub fn decode_image_usage(raw: &RawValue) -> Result<Option<ImageUsageMetadata>, GeminiImageError> {
    let decoded: ResponseJson =
        serde_json::from_str(raw.get()).map_err(|source| GeminiImageError::Shape {
            path: "usageMetadata".to_owned(),
            source,
        })?;
    let ResponseJson::Object(fields) = decoded else {
        return Err(GeminiImageError::UsageShape);
    };
    if fields.is_empty() {
        return Ok(None);
    }
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    serde_path_to_error::deserialize(&mut deserializer)
        .map(Some)
        .map_err(|error| GeminiImageError::Shape {
            path: error.path().to_string(),
            source: error.into_inner(),
        })
}

// serde's derive also reads a struct from a JSON array by position, and LiteLLM rejects a non-object
fn require_object(bytes: &[u8]) -> Result<(), GeminiImageError> {
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'{') {
        return Ok(());
    }
    Err(GeminiImageError::Shape {
        path: "the document root".to_owned(),
        source: JsonError::custom("the value is not a JSON object"),
    })
}

// only a body that does not parse keeps the reply's status and headers; a shape error carries neither
fn decode_json<T>(reply: &UpstreamResponse, bytes: &[u8]) -> Result<T, GeminiImageError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let path = error.path().to_string();
        let source = error.into_inner();
        match source.classify() {
            Category::Syntax | Category::Eof => GeminiImageError::NotJson {
                status: reply.status,
                headers: Box::new(reply.headers.clone()),
                source,
            },
            Category::Data | Category::Io => GeminiImageError::Shape { path, source },
        }
    })?;
    deserializer
        .end()
        .map_err(|source| GeminiImageError::NotJson {
            status: reply.status,
            headers: Box::new(reply.headers.clone()),
            source,
        })?;
    Ok(value)
}
