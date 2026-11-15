use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;
use thiserror::Error;

use super::events::ModelFields;

#[derive(Debug, Error)]
pub enum GenerationMetadataError {
    #[error("additionalModelFields is not valid JSON")]
    Malformed(#[source] serde_json::Error),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Document {
    Fields(BTreeMap<String, Field>),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Field {
    Text(String),
    Other(IgnoredAny),
}

// A JSON value that is not an object, or a stage that is not a string, carries no stage.
pub fn decode_generation_metadata(fields: &str) -> Result<Option<String>, GenerationMetadataError> {
    let document: Document =
        serde_json::from_str(fields).map_err(GenerationMetadataError::Malformed)?;
    match document {
        Document::Fields(mut entries) => match entries.remove("generationStage") {
            Some(Field::Text(stage)) => Ok(Some(stage)),
            Some(Field::Other(_)) | None => Ok(None),
        },
        Document::Other(_) => Ok(None),
    }
}

// LiteLLM treats unreadable metadata as no stage, so the error stops here.
#[must_use]
pub fn parse_generation_stage(fields: Option<&ModelFields>) -> Option<String> {
    match fields {
        Some(ModelFields::Text(text)) => decode_generation_metadata(text).ok().flatten(),
        Some(ModelFields::Other(_)) | None => None,
    }
}
