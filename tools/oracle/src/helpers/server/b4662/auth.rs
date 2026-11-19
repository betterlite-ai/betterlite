use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_mcp::server_record::MCPServer;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::mcp::oauth_caches::MCPOAuth2TokenCache;
use betterlite_server::mcp::oauth_caches::auth::{McpAuthOverride, McpAuthValue};
use betterlite_server::mcp::oauth_caches::resolve_mcp_auth;
use betterlite_server::mcp::oauth_caches::resolved_token_header;
use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::M2mTokenEndpointPost;

use super::support::{self, FixedClock, OutcomeSpec, ScriptedPost, ServerSpec};

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum AuthOverrideSpec {
    Text(String),
    Headers(BTreeMap<String, String>),
}

#[derive(Debug, Deserialize)]
pub(super) struct ResolveInput {
    server: ServerSpec,
    mcp_auth_header: Option<AuthOverrideSpec>,
    #[serde(default)]
    outcomes: Vec<OutcomeSpec>,
}

#[derive(Debug, Deserialize)]
pub(super) struct HeaderInput {
    server: ServerSpec,
    mcp_auth_header: Option<AuthOverrideSpec>,
}

pub(super) async fn resolve(input: &ResolveInput) -> Result<Value, Box<HelperError>> {
    let server: MCPServer = serde_json::from_value(input.server.0.clone())
        .map_err(|error| support::value_error(error.to_string()))?;
    let scripted = Arc::new(ScriptedPost::new(&input.outcomes)?);
    let poster: Arc<dyn M2mTokenEndpointPost> = Arc::<ScriptedPost>::clone(&scripted);
    let cache = MCPOAuth2TokenCache::new(poster, Arc::new(FixedClock));
    let header = override_of(input.mcp_auth_header.as_ref());
    let value = resolve_mcp_auth(&server, header.as_ref(), &cache)
        .await
        .map_err(|error| support::value_error(error.to_string()))?;
    Ok(json!({"auth": auth_value(value.as_ref()), "posts": scripted.post_count()}))
}

pub(super) fn header(input: &HeaderInput) -> Result<Value, Box<HelperError>> {
    let server: MCPServer = serde_json::from_value(input.server.0.clone())
        .map_err(|error| support::value_error(error.to_string()))?;
    let header = override_of(input.mcp_auth_header.as_ref());
    Ok(json!({"header": resolved_token_header(&server, header.as_ref())}))
}

fn override_of(spec: Option<&AuthOverrideSpec>) -> Option<McpAuthOverride> {
    match spec? {
        AuthOverrideSpec::Text(text) => {
            Some(McpAuthOverride::Text(SecretString::from(text.clone())))
        }
        AuthOverrideSpec::Headers(headers) => Some(McpAuthOverride::Headers(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
                .collect(),
        )),
    }
}

fn auth_value(value: Option<&McpAuthValue>) -> Value {
    match value {
        None => Value::Null,
        Some(McpAuthValue::Text(text)) => Value::String(text.expose_secret().clone()),
        Some(McpAuthValue::Headers(headers)) => json!(
            headers
                .iter()
                .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
                .collect::<BTreeMap<_, _>>()
        ),
    }
}
