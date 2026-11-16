use std::fmt::{Formatter, Result as FmtResult};

use betterlite_http::proxy::UpstreamResponse;
use serde::Deserialize;
use serde::de::Error as _;
use serde::de::value::MapAccessDeserializer;
use serde::de::{MapAccess, Visitor};
use serde_json::Deserializer;
use serde_json::Error as JsonError;
use serde_json::value::RawValue;

use super::error::ImagenGenerationError;
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

// transform_image_generation_response reads the reply as a dict and keeps only these prediction fields
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagenReply {
    #[serde(default)]
    pub predictions: Vec<ImagenPrediction>,
}

// the key's presence decides whether an image is appended, so null is Some(None) and an absent key is None
#[derive(Debug)]
pub struct ImagenPrediction {
    pub bytes_base64_encoded: Option<Option<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PredictionFields {
    #[serde(
        default,
        deserialize_with = "serde_with::rust::double_option::deserialize"
    )]
    bytes_base64_encoded: Option<Option<String>>,
}

// serde's derive also reads a struct from a JSON array by position, and LiteLLM's `in` test on a list finds no key
struct MapOnly;

impl<'de> Visitor<'de> for MapOnly {
    type Value = ImagenPrediction;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<ImagenPrediction, A::Error> {
        let fields = PredictionFields::deserialize(MapAccessDeserializer::new(map))?;
        Ok(ImagenPrediction {
            bytes_base64_encoded: fields.bytes_base64_encoded,
        })
    }
}

impl<'de> Deserialize<'de> for ImagenPrediction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(MapOnly)
    }
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
    serde_path_to_error::deserialize(&mut Deserializer::from_slice(&reply.body)).map_err(|error| {
        ImagenGenerationError::Shape {
            path: error.path().to_string(),
            source: error.into_inner(),
        }
    })
}
