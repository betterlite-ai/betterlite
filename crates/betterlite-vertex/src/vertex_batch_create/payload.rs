use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde::de::Error as _;

use super::error::VertexBatchCreateError;

mod tests;

pub const VERTEX_BATCH_PAYLOAD_LIMIT: usize = 4 * 1024 * 1024;

// LiteLLM's _VertexEndpointResponse: only the first deployed model is read, and a null list or model takes the absent default
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct VertexEndpointResponse {
    pub deployed_models: Option<Vec<VertexDeployedModel>>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct VertexDeployedModel {
    pub model: Option<String>,
}

// one bounded decoder serves the batch job and endpoint envelopes, and B1373's lifecycle calls read their jobs through it too
pub fn decode_vertex_batch_payload<Payload>(bytes: &[u8]) -> Result<Payload, VertexBatchCreateError>
where
    Payload: DeserializeOwned,
{
    if bytes.len() > VERTEX_BATCH_PAYLOAD_LIMIT {
        return Err(VertexBatchCreateError::PayloadTooLarge {
            limit: VERTEX_BATCH_PAYLOAD_LIMIT,
        });
    }
    // serde accepts a sequence for a struct, so the root type is checked before the typed parse
    if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) != Some(&b'{') {
        return Err(VertexBatchCreateError::Decode {
            path: "the document root".to_owned(),
            source: serde_json::Error::custom("the value is not a JSON object"),
        });
    }
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let payload = serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
        VertexBatchCreateError::Decode {
            path: error.path().to_string(),
            source: error.into_inner(),
        }
    })?;
    deserializer
        .end()
        .map_err(|source| VertexBatchCreateError::Decode {
            path: "the document root".to_owned(),
            source,
        })?;
    Ok(payload)
}
