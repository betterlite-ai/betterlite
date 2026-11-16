use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde::de::{Deserializer, Error as _};

use super::error::VertexDiscoverySearchError;

pub const DEFAULT_COLLECTION_ID: &str = "default_collection";

fn default_collection() -> Option<String> {
    Some(DEFAULT_COLLECTION_ID.to_owned())
}

// compatibility.md: a null key takes the same default as an absent one
fn collection_id<'de, Decoder>(deserializer: Decoder) -> Result<Option<String>, Decoder::Error>
where
    Decoder: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.or_else(default_collection))
}

// the per-deployment litellm_params keys that pick the Discovery Engine serving config
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VertexSearchOptions {
    #[serde(
        default = "default_collection",
        deserialize_with = "collection_id"
    )]
    pub vertex_collection_id: Option<String>,
    pub vertex_engine_id: Option<String>,
    pub vector_store_id: Option<String>,
}

impl Default for VertexSearchOptions {
    fn default() -> Self {
        Self {
            vertex_collection_id: default_collection(),
            vertex_engine_id: None,
            vector_store_id: None,
        }
    }
}

impl VertexSearchOptions {
    pub fn from_params(
        params: &BTreeMap<String, Json>,
    ) -> Result<Self, VertexDiscoverySearchError> {
        let encoded = serde_json::to_vec(params).map_err(VertexDiscoverySearchError::Options)?;
        let mut decoder = serde_json::Deserializer::from_slice(&encoded);
        serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
            let path = error.path().to_string();
            VertexDiscoverySearchError::Options(serde_json::Error::custom(format!(
                "{path}: {}",
                error.into_inner()
            )))
        })
    }
}
