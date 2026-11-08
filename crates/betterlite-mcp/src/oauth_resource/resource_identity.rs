mod tests;

use secrecy::SecretString;
use thiserror::Error;
use tracing::warn;

use crate::credential_contracts::token_endpoint_auth::{
    TokenEndpointAuthConfigError, TokenEndpointClientAuth, build_token_client_auth,
    normalize_token_auth_method,
};

use super::base_url::strip_default_port;
use super::python_url;

pub const UPSTREAM_RESOURCE_AUTO: &str = "auto";

#[derive(Debug, Clone, Copy)]
pub struct UpstreamServer<'server> {
    pub upstream_resource: Option<&'server str>,
    pub url: Option<&'server str>,
    pub server_id: &'server str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UrlIdentityError {
    #[error(transparent)]
    Parse(#[from] python_url::UrlParseError),
}

pub fn canonicalize_url_identity(url: &str) -> Result<String, UrlIdentityError> {
    let parsed = python_url::parse(url)?;
    let scheme = parsed.scheme.to_lowercase();
    let authority = parsed
        .netloc
        .rsplit_once('@')
        .map_or(parsed.netloc.as_str(), |(_userinfo, rest)| rest);
    let netloc = strip_default_port(&scheme, authority);
    let path = parsed.path.trim_end_matches('/');
    Ok(python_url::join(&scheme, &netloc, path, "", ""))
}

pub fn canonical_resource_uri(url: &str) -> Result<Option<String>, UrlIdentityError> {
    let parsed = python_url::parse(url)?;
    if parsed.scheme.is_empty() || parsed.netloc.is_empty() {
        return Ok(None);
    }
    Ok(Some(canonicalize_url_identity(url)?))
}

#[must_use]
pub fn resolve_upstream_resource(server: &UpstreamServer<'_>) -> Option<String> {
    let configured = server.upstream_resource.unwrap_or_default().trim();
    if configured.is_empty() {
        return None;
    }
    if !configured.eq_ignore_ascii_case(UPSTREAM_RESOURCE_AUTO) {
        return Some(configured.to_owned());
    }
    let Some(server_url) = server.url.filter(|value| !value.is_empty()) else {
        warn!(
            server_id = server.server_id,
            "upstream_resource=auto needs a server url to derive an RFC 8707 resource identifier; omitting it"
        );
        return None;
    };
    match canonical_resource_uri(server_url) {
        Ok(Some(resource)) => Some(resource),
        Ok(None) => {
            warn!(
                server_id = server.server_id,
                "the server url is not an absolute URI, so upstream_resource=auto derives nothing; omitting it"
            );
            None
        }
        Err(error) => {
            warn!(
                server_id = server.server_id,
                %error,
                "the server url could not be parsed, so upstream_resource=auto derives nothing; omitting it"
            );
            None
        }
    }
}

pub fn build_upstream_token_request(
    server: &UpstreamServer<'_>,
    auth_method: Option<&str>,
    client_id: Option<&str>,
    client_secret: Option<&SecretString>,
) -> Result<TokenEndpointClientAuth, TokenEndpointAuthConfigError> {
    let method = auth_method.and_then(normalize_token_auth_method);
    let mut client_auth = build_token_client_auth(method, client_id, client_secret)?;
    if let Some(resource) = resolve_upstream_resource(server) {
        client_auth
            .body
            .insert("resource".to_owned(), SecretString::from(resource));
    }
    Ok(client_auth)
}

#[must_use]
pub fn redact_mcp_resource_url(url: Option<&str>) -> Option<String> {
    let raw_url = url.filter(|value| !value.is_empty())?;
    let parsed = python_url::parse(raw_url).ok()?;
    let mut netloc = python_url::netloc::hostname(&parsed.netloc)?;
    let port_value = python_url::netloc::port(&parsed.netloc).ok()?;
    if let Some(port) = port_value.filter(|value| *value != 0) {
        netloc = format!("{netloc}:{port}");
    }
    let redacted = python_url::join(&parsed.scheme, &netloc, "", "", "");
    if redacted.is_empty() {
        None
    } else {
        Some(redacted)
    }
}
