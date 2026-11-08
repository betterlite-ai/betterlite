use betterlite_prompts::arguments::json::{Json, Object};
use jsonwebtoken::jwk::Jwk;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct DiscoveryDocument {
    #[serde(default)]
    pub jwks_uri: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct JwksDocument {
    #[serde(default)]
    pub keys: Option<Vec<Jwk>>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct IntrospectionResponse {
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(flatten)]
    pub claims: Object,
}

impl IntrospectionResponse {
    #[must_use]
    pub fn into_claims(self) -> Object {
        let mut claims = self.claims;
        if let Some(active) = self.active {
            claims.insert("active".to_owned(), Json::Bool(active));
        }
        claims
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdpDocument {
    Discovery(DiscoveryDocument),
    Jwks(JwksDocument),
    Introspection(IntrospectionResponse),
}

#[derive(Debug, Error)]
pub enum IdpDocumentError {
    #[error("the document is not a JSON object: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
}

pub fn decode_idp_document(bytes: &[u8]) -> Result<IdpDocument, IdpDocumentError> {
    let object: Object =
        serde_json::from_slice(bytes).map_err(|source| IdpDocumentError::Json { source })?;
    if object.contains_key("keys") {
        let document =
            serde_json::from_slice(bytes).map_err(|source| IdpDocumentError::Json { source })?;
        return Ok(IdpDocument::Jwks(document));
    }
    if object.contains_key("jwks_uri") {
        let document =
            serde_json::from_slice(bytes).map_err(|source| IdpDocumentError::Json { source })?;
        return Ok(IdpDocument::Discovery(document));
    }
    let document =
        serde_json::from_slice(bytes).map_err(|source| IdpDocumentError::Json { source })?;
    Ok(IdpDocument::Introspection(document))
}
