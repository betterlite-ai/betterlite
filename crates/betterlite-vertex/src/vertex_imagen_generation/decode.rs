use betterlite_http::proxy::UpstreamResponse;
use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Deserializer;
use serde_json::Error as JsonError;
use serde_json::error::Category;
use serde_json::value::RawValue;
use serde_path_to_error::Error as PathError;

use super::error::ImagenGenerationError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

// transform_image_generation_response reads the reply as a dict and keeps only these prediction fields
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagenReply {
    pub predictions: Option<Vec<ImagenPrediction>>,
}

// the key's presence decides whether an image is appended, so absent and null stay None
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagenPrediction {
    pub bytes_base64_encoded: Option<String>,
}

pub fn decode_imagen_generation(
    reply: &UpstreamResponse,
) -> Result<ImagenReply, ImagenGenerationError> {
    if reply.body.len() > MAX_RESPONSE_BYTES {
        return Err(ImagenGenerationError::TooLarge {
            size: reply.body.len(),
        });
    }
    // serde's derive also reads a struct from a JSON array by position, and LiteLLM calls .get on the reply
    if reply.body.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(non_object_error(reply));
    }
    let mut deserializer = Deserializer::from_slice(&reply.body);
    let decoded = serde_path_to_error::deserialize(&mut deserializer)
        .map_err(|error| reply_error(reply, error))?;
    deserializer
        .end()
        .map_err(|source| ImagenGenerationError::NotJson {
            status: reply.status,
            headers: Box::new(reply.headers.clone()),
            source,
        })?;
    Ok(decoded)
}

// a body json() rejects keeps the reply's status and headers; a valid document that is not an object names its root
fn non_object_error(reply: &UpstreamResponse) -> ImagenGenerationError {
    if let Err(source) = serde_json::from_slice::<&RawValue>(&reply.body) {
        return ImagenGenerationError::NotJson {
            status: reply.status,
            headers: Box::new(reply.headers.clone()),
            source,
        };
    }
    ImagenGenerationError::Shape {
        path: "the document root".to_owned(),
        source: JsonError::custom("the value is not a JSON object"),
    }
}

// only a body that does not parse keeps the reply's status and headers; a shape error carries neither
fn reply_error(reply: &UpstreamResponse, error: PathError<JsonError>) -> ImagenGenerationError {
    let path = error.path().to_string();
    let source = error.into_inner();
    match source.classify() {
        Category::Syntax | Category::Eof => ImagenGenerationError::NotJson {
            status: reply.status,
            headers: Box::new(reply.headers.clone()),
            source,
        },
        Category::Data | Category::Io => ImagenGenerationError::Shape { path, source },
    }
}
