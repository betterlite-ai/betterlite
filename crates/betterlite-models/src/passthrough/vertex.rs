#![expect(
    clippy::struct_field_names,
    dead_code,
    reason = "the fields keep LiteLLM's config key names, and a later credential-resolution task reads the record"
)]

use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

// LiteLLM's VERTEX_CREDENTIALS_TYPES is str | dict[str, str]: a file path or inline JSON text, or the fields themselves.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum VertexCredentials {
    Text(String),
    Fields(BTreeMap<String, SecretString>),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub(crate) struct VertexPassThroughCredentials {
    pub(crate) vertex_project: Option<String>,
    pub(crate) vertex_location: Option<String>,
    pub(crate) vertex_credentials: Option<VertexCredentials>,
}
