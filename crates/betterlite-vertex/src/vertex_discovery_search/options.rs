use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde::de::Error as _;

use super::error::VertexDiscoverySearchError;

pub const DEFAULT_COLLECTION_ID: &str = "default_collection";

// the per-deployment litellm_params keys that pick the Discovery Engine serving config
#[derive(Debug, Default, Deserialize)]
pub struct RawSearchOptions {
    vertex_collection_id: Option<String>,
    vertex_engine_id: Option<String>,
    vector_store_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexSearchOptions {
    pub vertex_collection_id: String,
    pub vertex_engine_id: Option<String>,
    pub vector_store_id: Option<String>,
}

// compatibility.md: a null key takes the same default as an absent one, resolved once here
impl From<RawSearchOptions> for VertexSearchOptions {
    fn from(raw: RawSearchOptions) -> Self {
        Self {
            vertex_collection_id: raw
                .vertex_collection_id
                .unwrap_or_else(|| DEFAULT_COLLECTION_ID.to_owned()),
            vertex_engine_id: raw.vertex_engine_id,
            vector_store_id: raw.vector_store_id,
        }
    }
}

impl Default for VertexSearchOptions {
    fn default() -> Self {
        Self::from(RawSearchOptions::default())
    }
}

impl VertexSearchOptions {
    pub fn from_params(
        params: &BTreeMap<String, Json>,
    ) -> Result<Self, VertexDiscoverySearchError> {
        let encoded = serde_json::to_vec(params).map_err(VertexDiscoverySearchError::Options)?;
        let mut decoder = serde_json::Deserializer::from_slice(&encoded);
        let raw: RawSearchOptions =
            serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
                let path = error.path().to_string();
                VertexDiscoverySearchError::Options(serde_json::Error::custom(format!(
                    "{path}: {}",
                    error.into_inner()
                )))
            })?;
        Ok(Self::from(raw))
    }
}
