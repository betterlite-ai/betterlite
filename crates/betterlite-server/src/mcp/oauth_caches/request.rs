use betterlite_db::mcp_stored_contracts::MCPTokenEndpointAuthMethod;
use betterlite_mcp::oauth_resource::resource_identity::{
    UpstreamServer, build_upstream_token_request,
};
use betterlite_mcp::server_record::MCPServer;
use secrecy::{ExposeSecret as _, SecretString};

use super::OAUTH2_TOKEN_CACHE_DEFAULT_TTL;
use super::error::OAuthCacheError;
use crate::mcp::outbound_credentials::client_credentials::grant::parse_expires_in;
use crate::mcp::outbound_credentials::client_credentials::token_post::ExpiresIn;

// LiteLLM reads a missing, null or empty client field as unusable and names every absent one in the error.
pub(super) fn token_request_inputs(
    server: &MCPServer,
) -> Result<(&str, &str, SecretString), OAuthCacheError> {
    let token_url = server.effective_token_url().filter(|url| !url.is_empty());
    let client_id = server
        .fields
        .client_id
        .as_deref()
        .filter(|value| !value.is_empty());
    let secret = server
        .fields
        .client_secret
        .as_ref()
        .map(|credential| credential.0.0.clone())
        .filter(|value| !value.expose_secret().is_empty());
    let has_client_id = client_id.is_some();
    let has_secret = secret.is_some();
    let has_token_url = token_url.is_some();
    let (Some(url), Some(id), Some(client_secret)) = (token_url, client_id, secret) else {
        return Err(OAuthCacheError::MissingFields {
            server_id: server.fields.server_id.clone(),
            client_id: has_client_id,
            client_secret: has_secret,
            token_url: has_token_url,
        });
    };
    Ok((url, id, client_secret))
}

pub(super) const fn auth_method_name(method: MCPTokenEndpointAuthMethod) -> &'static str {
    match method {
        MCPTokenEndpointAuthMethod::ClientSecretBasic => "client_secret_basic",
        MCPTokenEndpointAuthMethod::ClientSecretPost => "client_secret_post",
    }
}

pub(super) struct TokenRequestForm {
    pub(super) form: Vec<(String, SecretString)>,
    pub(super) headers: Vec<(String, SecretString)>,
}

// The wire order is LiteLLM's: grant_type, the client auth body, then scope, matching build_upstream_oauth2_token_request.
pub(super) fn token_request_form(
    server: &MCPServer,
    client_id: &str,
    secret: &SecretString,
) -> Option<TokenRequestForm> {
    let upstream = UpstreamServer {
        upstream_resource: server.fields.upstream_resource.as_deref(),
        url: server.fields.url.as_deref(),
        server_id: server.fields.server_id.as_str(),
    };
    let method = server
        .fields
        .token_endpoint_auth_method
        .map(auth_method_name);
    let client_auth =
        build_upstream_token_request(&upstream, method, Some(client_id), Some(secret)).ok()?;
    let capacity = client_auth.body.len().saturating_add(2);
    let mut form: Vec<(String, SecretString)> = Vec::with_capacity(capacity);
    form.push((
        "grant_type".to_owned(),
        SecretString::from("client_credentials".to_owned()),
    ));
    form.extend(client_auth.body);
    if let Some(scopes) = server
        .fields
        .scopes
        .as_deref()
        .filter(|scopes| !scopes.is_empty())
    {
        form.push(("scope".to_owned(), SecretString::from(scopes.join(" "))));
    }
    Some(TokenRequestForm {
        form,
        headers: client_auth.headers.into_iter().collect(),
    })
}

pub(super) fn expires_in_seconds(raw: Option<&ExpiresIn>) -> i64 {
    match raw {
        None => OAUTH2_TOKEN_CACHE_DEFAULT_TTL,
        Some(ExpiresIn::Flag(flag)) => i64::from(*flag),
        Some(ExpiresIn::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(num_traits::cast::<f64, i64>))
            .unwrap_or(OAUTH2_TOKEN_CACHE_DEFAULT_TTL),
        Some(other) => parse_expires_in(Some(other)).unwrap_or(OAUTH2_TOKEN_CACHE_DEFAULT_TTL),
    }
}
