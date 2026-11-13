use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_store_contracts::config::Params;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RequestMetadataError {
    #[error("the {field} keys must be strings")]
    NonStringKey { field: &'static str },
}

// LiteLLM reads the request metadata from litellm_metadata, then metadata, and an empty dict still wins
#[must_use]
pub fn vector_store_request_metadata(kwargs: &Params) -> Params {
    for name in ["litellm_metadata", "metadata"] {
        if let Some(Json::Object(metadata)) = kwargs.get(name) {
            return metadata
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
        }
    }
    Params::new()
}

pub fn metadata_map(
    entries: Vec<(Json, Json)>,
    field: &'static str,
) -> Result<Params, RequestMetadataError> {
    let mut metadata = Params::new();
    for (key, value) in entries {
        let Json::Text(text) = key else {
            return Err(RequestMetadataError::NonStringKey { field });
        };
        metadata.insert(text, value);
    }
    Ok(metadata)
}
