use serde::{Deserialize, Serialize};

pub const OPENAPI_REGISTRY_JSON: &str = include_str!("../../data/openapi_registry.json");

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenApiRegistry {
    #[serde(default)]
    pub apis: Vec<OpenApiEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenApiEntry {
    pub name: String,
    pub title: String,
    pub description: String,
    pub icon_url: String,
    pub spec_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthMetadata>,
    pub key_tools: Vec<KeyTool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthMetadata {
    pub authorization_url: String,
    pub token_url: String,
    pub pkce: bool,
    pub docs_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyTool {
    pub name: String,
    pub description: String,
}

#[must_use]
pub fn openapi_registry(source: &str) -> OpenApiRegistry {
    match serde_json::from_str(source) {
        Ok(registry) => registry,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to load OpenAPI registry");
            OpenApiRegistry::default()
        }
    }
}

mod tests;
