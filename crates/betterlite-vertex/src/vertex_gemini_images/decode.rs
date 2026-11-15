use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Json as ResponseJson;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Error as JsonError;
use serde_json::error::Category;

use super::error::GeminiImageError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

// transform_image_generation_response reads the reply as a dict; only the fields it consumes are declared
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GoogleImageResponse {
    pub candidates: Option<Vec<GoogleImageCandidate>>,
    pub usage_metadata: Option<ResponseJson>,
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
    require_object(reply, &reply.body)?;
    decode_json(reply, &reply.body)
}

// serde's derive also reads a struct from a JSON array by position, and LiteLLM rejects a non-object
fn require_object(reply: &UpstreamResponse, bytes: &[u8]) -> Result<(), GeminiImageError> {
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'{') {
        return Ok(());
    }
    Err(GeminiImageError::Shape {
        path: "the document root".to_owned(),
        source: JsonError::custom("the value is not a JSON object"),
    })
}

// a body httpx cannot parse keeps the reply's status and headers, as LiteLLM's get_error_class does;
// a body that parses but has the wrong shape is raised locally and carries neither
fn decode_json<T>(reply: &UpstreamResponse, bytes: &[u8]) -> Result<T, GeminiImageError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        let path = error.path().to_string();
        let source = error.into_inner();
        match source.classify() {
            Category::Syntax | Category::Eof => unparseable(reply, source),
            Category::Data | Category::Io => GeminiImageError::Shape { path, source },
        }
    })?;
    deserializer
        .end()
        .map_err(|source| unparseable(reply, source))?;
    Ok(value)
}

fn unparseable(reply: &UpstreamResponse, source: JsonError) -> GeminiImageError {
    GeminiImageError::NotJson {
        status: reply.status,
        headers: Box::new(reply.headers.clone()),
        source,
    }
}
