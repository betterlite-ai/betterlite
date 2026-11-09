use crate::oauth_resource::python_url::{self, ParsedUrl};
use crate::server_oauth::basics::blank_to_none;
use crate::server_oauth::issuer::issuer_matches;
use crate::server_record::MCPOAuthMetadata;

use super::document::{MetadataDocument, document_scopes, document_text};
use super::fetch::{MetadataFetcher, fetch_oauth_discovery_url};
use super::outcome::DiscoveryStatus;
use super::redaction::UNPARSABLE_URL;
use super::settings::DiscoverySettings;
use crate::oauth_resource::resource_identity::redact_mcp_resource_url;

pub const AZURE_ENTRA_HOSTS: [&str; 3] = [
    "login.microsoftonline.com",
    "login.microsoftonline.us",
    "login.chinacloudapi.cn",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthorizationLookup {
    pub metadata: Option<MCPOAuthMetadata>,
    pub statuses: Vec<DiscoveryStatus>,
}

pub async fn fetch_authorization_metadata<F>(
    fetcher: &F,
    issuers: &[String],
    settings: DiscoverySettings,
) -> AuthorizationLookup
where
    F: MetadataFetcher,
{
    let mut statuses = Vec::new();
    for issuer in issuers {
        let lookup = fetch_single_issuer_metadata(fetcher, issuer, None, settings).await;
        statuses.extend(lookup.statuses);
        if let Some(metadata) = lookup.metadata {
            return AuthorizationLookup {
                metadata: Some(metadata),
                statuses,
            };
        }
    }
    AuthorizationLookup {
        metadata: None,
        statuses,
    }
}

pub async fn fetch_single_issuer_metadata<F>(
    fetcher: &F,
    issuer_url: &str,
    require_issuer: Option<&str>,
    settings: DiscoverySettings,
) -> AuthorizationLookup
where
    F: MetadataFetcher,
{
    let Ok(parsed) = python_url::parse(issuer_url) else {
        return AuthorizationLookup::default();
    };
    if parsed.scheme.is_empty() || parsed.netloc.is_empty() {
        return AuthorizationLookup::default();
    }
    let (metadata, statuses) =
        first_issuer_metadata(fetcher, &parsed, issuer_url, require_issuer, settings).await;
    AuthorizationLookup {
        metadata: metadata.or_else(|| azure_metadata(&parsed)),
        statuses,
    }
}

async fn first_issuer_metadata<F>(
    fetcher: &F,
    parsed: &ParsedUrl,
    issuer_url: &str,
    require_issuer: Option<&str>,
    settings: DiscoverySettings,
) -> (Option<MCPOAuthMetadata>, Vec<DiscoveryStatus>)
where
    F: MetadataFetcher,
{
    let mut statuses = Vec::new();
    for url in authorization_candidates(parsed, issuer_url) {
        let document =
            match fetch_oauth_discovery_url(fetcher, &url, settings.metadata_timeout).await {
                Ok(document) => document,
                Err(status) => {
                    statuses.push(status);
                    continue;
                }
            };
        if let Some(required) = require_issuer
            && !issuer_matches(document.issuer.as_ref(), required)
        {
            statuses.push(DiscoveryStatus::IssuerMismatch);
            let origin =
                redact_mcp_resource_url(Some(&url)).unwrap_or_else(|| UNPARSABLE_URL.to_owned());
            tracing::warn!(
                "the MCP authorization server metadata at {origin} does not self-attest the configured issuer {required} (RFC 8414 section 3.3); rejecting it"
            );
            continue;
        }
        let metadata = authorization_metadata(&document);
        if metadata.scopes.is_some()
            || metadata.authorization_url.is_some()
            || metadata.token_url.is_some()
            || metadata.registration_url.is_some()
        {
            return (Some(metadata), statuses);
        }
    }
    (None, statuses)
}

fn authorization_metadata(document: &MetadataDocument) -> MCPOAuthMetadata {
    MCPOAuthMetadata {
        scopes: document_scopes(document.scopes_supported.as_ref()),
        authorization_url: blank_to_none(document.authorization_endpoint.as_deref())
            .map(str::to_owned),
        token_url: blank_to_none(document.token_endpoint.as_deref()).map(str::to_owned),
        registration_url: blank_to_none(document.registration_endpoint.as_deref())
            .map(str::to_owned),
        discovered_issuer: blank_to_none(document_text(document.issuer.as_ref()))
            .map(str::to_owned),
        from_origin_fallback: false,
    }
}

fn authorization_candidates(parsed: &ParsedUrl, issuer_url: &str) -> Vec<String> {
    let base = python_url::join(&parsed.scheme, &parsed.netloc, "", "", "");
    let path = parsed.path.trim_matches('/');
    let issuer = issuer_url.trim_end_matches('/');
    let mut candidates = Vec::new();
    if !path.is_empty() {
        candidates.push(format!(
            "{base}/.well-known/oauth-authorization-server/{path}"
        ));
        candidates.push(format!("{base}/.well-known/openid-configuration/{path}"));
        candidates.push(format!("{issuer}/.well-known/openid-configuration"));
    }
    candidates.push(format!("{base}/.well-known/oauth-authorization-server"));
    candidates.push(format!("{base}/.well-known/openid-configuration"));
    candidates.push(issuer.to_owned());
    candidates
}

fn azure_metadata(parsed: &ParsedUrl) -> Option<MCPOAuthMetadata> {
    if !AZURE_ENTRA_HOSTS.contains(&parsed.netloc.as_str()) {
        return None;
    }
    let parts: Vec<&str> = parsed
        .path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 2 || parts.get(1) != Some(&"v2.0") {
        return None;
    }
    let tenant = parts.first()?;
    let base = python_url::join(&parsed.scheme, &parsed.netloc, tenant, "", "");
    Some(MCPOAuthMetadata {
        scopes: None,
        authorization_url: Some(format!("{base}/oauth2/v2.0/authorize")),
        token_url: Some(format!("{base}/oauth2/v2.0/token")),
        registration_url: None,
        discovered_issuer: None,
        from_origin_fallback: false,
    })
}
