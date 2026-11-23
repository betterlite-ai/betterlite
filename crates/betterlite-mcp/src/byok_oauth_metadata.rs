mod tests;

use serde::Serialize;

use crate::oauth_resource::python_url::{self, UrlParseError};

// _byok_authorization_server_response: the six keys in LiteLLM's order, with no registration_endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorizationServerDocument {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub response_types_supported: [&'static str; 1],
    pub grant_types_supported: [&'static str; 1],
    // LiteLLM spells the key out; the field name has to stay under the repository's 30 character limit.
    #[serde(rename = "code_challenge_methods_supported")]
    pub code_challenge_methods: [&'static str; 1],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProtectedResourceDocument {
    pub resource: String,
    pub authorization_servers: [String; 1],
}

#[must_use]
pub fn authorization_server_document(base_url: &str, issuer: &str) -> AuthorizationServerDocument {
    AuthorizationServerDocument {
        issuer: issuer.to_owned(),
        authorization_endpoint: format!("{base_url}/v1/mcp/oauth/authorize"),
        token_endpoint: format!("{base_url}/v1/mcp/oauth/token"),
        response_types_supported: ["code"],
        grant_types_supported: ["authorization_code"],
        code_challenge_methods: ["S256"],
    }
}

pub fn protected_resource_document(
    base_url: &str,
) -> Result<ProtectedResourceDocument, UrlParseError> {
    let parsed = python_url::parse(base_url)?;
    Ok(ProtectedResourceDocument {
        resource: format!("{}://{}", parsed.scheme, parsed.netloc),
        authorization_servers: [format!("{base_url}/v1/mcp/oauth")],
    })
}

pub fn prefixed_base_url(base_url: &str, prefix: &str) -> Result<String, UrlParseError> {
    python_url::parse(base_url)
        .map(|parsed| format!("{}://{}{prefix}", parsed.scheme, parsed.netloc))
}
