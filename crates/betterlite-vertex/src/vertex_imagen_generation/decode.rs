use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::{Extra, Json};
use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Deserializer;
use serde_json::Error as JsonError;
use serde_json::value::RawValue;

use super::error::ImagenGenerationError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

// transform_image_generation_response reads the reply as a dict and keeps only these prediction fields
#[derive(Debug, Default)]
pub struct ImagenReply {
    pub predictions: Vec<ImagenPrediction>,
}

// the key's presence decides whether an image is appended, so null is Some(None) and an absent key is None
#[derive(Debug, Default)]
pub struct ImagenPrediction {
    pub bytes_base64_encoded: Option<Option<String>>,
}

// LiteLLM calls .get on each prediction, so a JSON array or null is refused here rather than read by position
#[derive(Debug, Deserialize)]
struct RawImagenReply {
    #[serde(default)]
    predictions: Vec<Extra>,
}

// LiteLLM's `in` test appends for any prediction holding the key, and a value that is no string is malformed
fn prediction_at(fields: &Extra, index: usize) -> Result<ImagenPrediction, ImagenGenerationError> {
    let Some(encoded) = fields.get("bytesBase64Encoded") else {
        return Ok(ImagenPrediction {
            bytes_base64_encoded: None,
        });
    };
    let value = match encoded {
        Json::Null => None,
        Json::Text(text) => Some(text.clone()),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(ImagenGenerationError::Shape {
                path: format!("predictions[{index}].bytesBase64Encoded"),
                source: JsonError::custom("the value is not a string"),
            });
        }
    };
    Ok(ImagenPrediction {
        bytes_base64_encoded: Some(value),
    })
}

pub fn decode_imagen_generation(
    reply: &UpstreamResponse,
) -> Result<ImagenReply, ImagenGenerationError> {
    if reply.body.len() > MAX_RESPONSE_BYTES {
        return Err(ImagenGenerationError::TooLarge {
            size: reply.body.len(),
        });
    }
    // a body json() rejects keeps the reply's status and headers, so the whole syntax is checked before any shape
    if let Err(source) = serde_json::from_slice::<&RawValue>(&reply.body) {
        return Err(ImagenGenerationError::NotJson {
            status: reply.status,
            headers: Box::new(reply.headers.clone()),
            source,
        });
    }
    // serde's derive also reads a struct from a JSON array by position, and LiteLLM calls .get on the reply
    if reply.body.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(ImagenGenerationError::Shape {
            path: "the document root".to_owned(),
            source: JsonError::custom("the value is not a JSON object"),
        });
    }
    let raw: RawImagenReply = serde_path_to_error::deserialize(&mut Deserializer::from_slice(
        &reply.body,
    ))
    .map_err(|error| ImagenGenerationError::Shape {
        path: error.path().to_string(),
        source: error.into_inner(),
    })?;
    let mut predictions = Vec::with_capacity(raw.predictions.len());
    for (index, fields) in raw.predictions.iter().enumerate() {
        predictions.push(prediction_at(fields, index)?);
    }
    Ok(ImagenReply { predictions })
}
