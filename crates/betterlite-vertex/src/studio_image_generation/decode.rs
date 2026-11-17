use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::decisions::Extra;
use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Error as JsonError;
use serde_json::value::RawValue;

use super::error::StudioImageError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::vertex_gemini_images::decode::GoogleImageCandidate;

// raw_response.json() hands LiteLLM one dict holding whichever of the two reply shapes the model sends
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StudioImageReply {
    pub candidates: Option<Vec<GoogleImageCandidate>>,
    // Imagen's branch reads .get on each prediction, so a prediction stays an object here
    pub predictions: Option<Vec<Extra>>,
    // the presence of the key decides whether usage is built, so the value stays unread
    pub usage_metadata: Option<Box<RawValue>>,
}

pub fn decode_studio_image(reply: &UpstreamResponse) -> Result<StudioImageReply, StudioImageError> {
    if reply.body.len() > MAX_RESPONSE_BYTES {
        return Err(StudioImageError::TooLarge {
            size: reply.body.len(),
        });
    }
    // a body that json() rejects keeps the reply's status and headers, so the syntax is checked first
    if let Err(source) = serde_json::from_slice::<&RawValue>(&reply.body) {
        return Err(StudioImageError::NotJson {
            status: reply.status,
            headers: Box::new(reply.headers.clone()),
            source,
        });
    }
    // serde's derive also reads a struct from a JSON array by position, and LiteLLM calls .get on the reply
    if reply.body.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(StudioImageError::Shape {
            path: "the document root".to_owned(),
            source: JsonError::custom("the value is not a JSON object"),
        });
    }
    serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_slice(&reply.body))
        .map_err(|error| StudioImageError::Shape {
            path: error.path().to_string(),
            source: error.into_inner(),
        })
}
