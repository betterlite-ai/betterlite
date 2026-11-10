use betterlite_db::mcp_stored_contracts::JsonValue;
use serde::Deserialize;
use serde::de::{Error as _, IgnoredAny};
use thiserror::Error;

use crate::server_oauth::merge::{ScopesInput, extract_scopes};

pub const MAX_METADATA_BYTES: usize = 0x0010_0000;

#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("the document is larger than {MAX_METADATA_BYTES} bytes")]
    TooLarge,
    #[error("the document is not a metadata document: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ScopesValue {
    Text(String),
    List(Vec<String>),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ServerEntry {
    Url(String),
    Other(IgnoredAny),
}

#[derive(Clone, Debug, Deserialize)]
pub struct MetadataDocument {
    #[serde(default)]
    pub authorization_servers: Option<Vec<ServerEntry>>,
    #[serde(default)]
    pub scopes_supported: Option<ScopesValue>,
    #[serde(default)]
    pub scopes: Option<ScopesValue>,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub issuer: Option<JsonValue>,
    #[serde(default)]
    pub authorization_endpoint: Option<String>,
    #[serde(default)]
    pub token_endpoint: Option<String>,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
}

pub fn decode_metadata_document(bytes: &[u8]) -> Result<MetadataDocument, DocumentError> {
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(DocumentError::TooLarge);
    }
    if bytes.trim_ascii_start().first() != Some(&b'{') {
        return Err(DocumentError::Json {
            source: serde_json::Error::custom("expected a JSON object"),
        });
    }
    serde_json::from_slice(bytes).map_err(|source| DocumentError::Json { source })
}

#[must_use]
pub fn document_scopes(value: Option<&ScopesValue>) -> Option<Vec<String>> {
    match value? {
        ScopesValue::Text(text) => extract_scopes(ScopesInput::Text(text)),
        ScopesValue::List(entries) => {
            let values: Vec<JsonValue> = entries
                .iter()
                .map(|entry| JsonValue::String(entry.clone()))
                .collect();
            extract_scopes(ScopesInput::List(&values))
        }
    }
}

#[must_use]
pub fn document_text(value: Option<&JsonValue>) -> Option<&str> {
    let JsonValue::String(text) = value? else {
        return None;
    };
    Some(text)
}
