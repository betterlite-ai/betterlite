pub mod callback;
pub mod payload;
pub mod timestamp;

use crate::otel::model::utils::coercion::OtelModelValue;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;

pub type MetadataMap = BTreeMap<String, OtelModelValue>;

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("invalid request metadata: {source}")]
    Decode {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("cannot project request metadata: {source}")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

pub fn decode<T>(text: &str) -> Result<T, MetadataError>
where
    T: DeserializeOwned,
{
    let mut decoder = serde_json::Deserializer::from_str(text);
    let value = serde_path_to_error::deserialize(&mut decoder)
        .map_err(|source| MetadataError::Decode { source })?;
    decoder.end()?;
    Ok(value)
}
