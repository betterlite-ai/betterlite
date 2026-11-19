use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreChunkingStrategy, VectorStoreExpirationPolicy,
};
use betterlite_openai::openai_base_config::metadata_filter_limit;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::error::WriteError;

// LiteLLM's VectorStoreCreateOptionalRequestParams: an absent key, a null and a value stay distinct
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateParams {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub name: Option<Option<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub file_ids: Option<Option<Vec<String>>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub expires_after: Option<Option<VectorStoreExpirationPolicy>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub chunking_strategy: Option<Option<VectorStoreChunkingStrategy>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub metadata: Option<Option<IndexMap<String, Json>>>,
}

impl UpdateParams {
    // dict(optional_params) keeps the keys that are there, a null included, and B668 cleans non-null metadata before extra_body runs last
    pub fn request_body(
        &self,
        extra_body: Option<&IndexMap<String, Json>>,
    ) -> Result<IndexMap<String, Json>, WriteError> {
        let mut body = IndexMap::new();
        insert(&mut body, "name", &self.name)?;
        insert(&mut body, "file_ids", &self.file_ids)?;
        insert(&mut body, "expires_after", &self.expires_after)?;
        insert(&mut body, "chunking_strategy", &self.chunking_strategy)?;
        insert(&mut body, "metadata", &self.metadata)?;
        if let Some(Some(metadata)) = &self.metadata {
            body.insert("metadata".to_owned(), cleaned_metadata(metadata)?);
        }
        if let Some(extra) = extra_body {
            for (key, value) in extra {
                body.insert(key.clone(), value.clone());
            }
        }
        Ok(body)
    }
}

fn insert<T: Serialize>(
    body: &mut IndexMap<String, Json>,
    key: &str,
    field: &Option<Option<T>>,
) -> Result<(), WriteError> {
    match field {
        None => {}
        Some(None) => {
            body.insert(key.to_owned(), Json::Null);
        }
        Some(Some(value)) => {
            let encoded = serde_json::to_value(value).map_err(WriteError::Encode)?;
            body.insert(
                key.to_owned(),
                serde_json::from_value(encoded).map_err(WriteError::Encode)?,
            );
        }
    }
    Ok(())
}

// B668's filter reads serde_json's value; the metadata map round-trips through it and comes back typed
fn cleaned_metadata(metadata: &IndexMap<String, Json>) -> Result<Json, WriteError> {
    let encoded = serde_json::to_value(metadata).map_err(WriteError::Encode)?;
    let map = serde_json::from_value(encoded).map_err(WriteError::Encode)?;
    let cleaned = metadata_filter_limit(Some(&map));
    let encoded = serde_json::to_value(&cleaned).map_err(WriteError::Encode)?;
    serde_json::from_value(encoded).map_err(WriteError::Encode)
}
