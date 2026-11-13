use super::authority::is_same_authority_metadata_url;
use super::authorization::fetch_authorization_metadata;
use super::fetch::{FetchedDocument, MetadataFetcher};
use super::flow::DiscoveryRequest;
use super::outcome::{DiscoveryReport, DiscoveryStatus, combine};
use super::redaction::{UNPARSABLE_URL, redacted_origin_list};
use super::resource::{attempt_well_known_discovery, fetch_resource_metadata};
use super::settings::DiscoverySettings;
use super::www_authenticate::{WwwAuthenticate, challenge_from_headers};
use crate::oauth_resource::python_url;
use crate::oauth_resource::resource_identity::redact_mcp_resource_url;

const NO_HINT_LOOKUP: &str = "no challenge-advertised resource metadata; well-known protected-resource lookup found no authorization servers";
const EMPTY_HINT: &str = "challenge-advertised resource metadata yielded no authorization servers";

pub(super) async fn discover_after_status_error<F>(
    fetcher: &F,
    settings: DiscoverySettings,
    request: DiscoveryRequest<'_>,
    response: &FetchedDocument,
    origin: &str,
) -> DiscoveryReport
where
    F: MetadataFetcher,
{
    let status_attempt = format!("GET {origin}: HTTP {}", response.status.as_u16());
    let challenge = challenge_from_headers(&response.headers);
    let mut statuses = vec![DiscoveryStatus::HttpError(response.status)];
    let mut stage = resource_stage(fetcher, settings, request, &challenge).await;
    statuses.append(&mut stage.statuses);
    let mut servers = stage.authorization_servers;
    let used_origin_fallback =
        request.allow_origin_fallback && origin_fallback(request.server_url, &mut servers);
    let mut fallback_attempt = None;
    let mut metadata = None;
    if !servers.is_empty() {
        let lookup = fetch_authorization_metadata(fetcher, &servers, settings).await;
        statuses.extend(lookup.statuses);
        metadata = lookup.metadata;
        if metadata.is_none() {
            fallback_attempt = Some(no_authorization_attempt(
                used_origin_fallback,
                &servers,
                origin,
            ));
        }
    }
    if let Some(found) = &mut metadata
        && used_origin_fallback
    {
        found.from_origin_fallback = true;
    }
    let mut attempts = vec![status_attempt];
    attempts.extend(stage.attempt);
    attempts.extend(fallback_attempt);
    let preferred_scopes = challenge.scopes.or(stage.scopes);
    DiscoveryReport {
        outcome: combine(metadata, preferred_scopes, statuses),
        attempts,
    }
}

struct ResourceStage {
    authorization_servers: Vec<String>,
    scopes: Option<Vec<String>>,
    attempt: Option<String>,
    statuses: Vec<DiscoveryStatus>,
}

async fn resource_stage<F>(
    fetcher: &F,
    settings: DiscoverySettings,
    request: DiscoveryRequest<'_>,
    challenge: &WwwAuthenticate,
) -> ResourceStage
where
    F: MetadataFetcher,
{
    let Some(hint) = challenge.resource_metadata_url.as_deref() else {
        return well_known_stage(fetcher, settings, request).await;
    };
    if !is_same_authority_metadata_url(hint, request.server_url) {
        return cross_origin_stage(hint, request.server_url);
    }
    hint_stage(fetcher, hint, settings).await
}

async fn well_known_stage<F>(
    fetcher: &F,
    settings: DiscoverySettings,
    request: DiscoveryRequest<'_>,
) -> ResourceStage
where
    F: MetadataFetcher,
{
    let lookup = attempt_well_known_discovery(fetcher, request.server_url, settings).await;
    let attempt = lookup
        .authorization_servers
        .is_empty()
        .then(|| String::from(NO_HINT_LOOKUP));
    ResourceStage {
        authorization_servers: lookup.authorization_servers,
        scopes: lookup.scopes,
        attempt,
        statuses: lookup.statuses,
    }
}

fn cross_origin_stage(hint: &str, server_url: &str) -> ResourceStage {
    let hint_origin =
        redact_mcp_resource_url(Some(hint)).unwrap_or_else(|| UNPARSABLE_URL.to_owned());
    let server_origin =
        redact_mcp_resource_url(Some(server_url)).unwrap_or_else(|| UNPARSABLE_URL.to_owned());
    tracing::warn!(
        "MCP OAuth discovery is not following a resource_metadata hint from {hint_origin} for server {server_origin}: the hint is not on the server's own authority"
    );
    ResourceStage {
        authorization_servers: Vec::new(),
        scopes: None,
        attempt: Some(String::from(EMPTY_HINT)),
        statuses: vec![DiscoveryStatus::CrossOrigin],
    }
}

async fn hint_stage<F>(fetcher: &F, hint: &str, settings: DiscoverySettings) -> ResourceStage
where
    F: MetadataFetcher,
{
    match fetch_resource_metadata(fetcher, hint, settings).await {
        Ok(resource) => {
            let attempt = resource
                .authorization_servers
                .is_empty()
                .then(|| String::from(EMPTY_HINT));
            ResourceStage {
                authorization_servers: resource.authorization_servers,
                scopes: resource.scopes_supported,
                attempt,
                statuses: Vec::new(),
            }
        }
        Err(status) => ResourceStage {
            authorization_servers: Vec::new(),
            scopes: None,
            attempt: Some(String::from(EMPTY_HINT)),
            statuses: vec![status],
        },
    }
}

fn origin_fallback(server_url: &str, servers: &mut Vec<String>) -> bool {
    if !servers.is_empty() {
        return false;
    }
    let Ok(parsed) = python_url::parse(server_url) else {
        return false;
    };
    if parsed.scheme.is_empty() || parsed.netloc.is_empty() {
        return false;
    }
    *servers = vec![python_url::join(&parsed.scheme, &parsed.netloc, "", "", "")];
    true
}

fn no_authorization_attempt(
    used_origin_fallback: bool,
    servers: &[String],
    origin: &str,
) -> String {
    if used_origin_fallback {
        return format!("origin fallback: no authorization server metadata at {origin}");
    }
    format!(
        "authorization server metadata fetch failed for: {}",
        redacted_origin_list(servers)
    )
}
