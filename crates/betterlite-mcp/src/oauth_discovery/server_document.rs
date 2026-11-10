use http::StatusCode;

use super::authorization::fetch_authorization_metadata;
use super::fetch::{FetchedDocument, MetadataFetcher};
use super::flow::DiscoveryRequest;
use super::outcome::{DiscoveryReport, combine};
use super::redaction::redacted_origin_list;
use super::resource::{ResourceLookup, attempt_well_known_discovery};
use super::settings::DiscoverySettings;

pub(super) async fn discover_from_server_document<F>(
    fetcher: &F,
    settings: DiscoverySettings,
    request: DiscoveryRequest<'_>,
    response: &FetchedDocument,
    origin: &str,
) -> DiscoveryReport
where
    F: MetadataFetcher,
{
    let ResourceLookup {
        authorization_servers,
        scopes,
        statuses,
    } = attempt_well_known_discovery(fetcher, request.server_url, settings).await;
    let authorization =
        fetch_authorization_metadata(fetcher, &authorization_servers, settings).await;
    let mut failure_statuses = statuses;
    failure_statuses.extend(authorization.statuses);
    if authorization.metadata.is_none()
        && scopes.is_none()
        && !authorization_servers.is_empty()
        && response.status == StatusCode::OK
    {
        tracing::warn!(
            "MCP OAuth discovery for {origin} received 200 OK without an RFC 9728 challenge and no discoverable authorization metadata"
        );
    }
    let mut attempts = vec![format!(
        "GET {origin}: HTTP {} (no RFC 9728 challenge)",
        response.status.as_u16()
    )];
    if authorization_servers.is_empty() {
        attempts.push(String::from(
            "well-known protected-resource lookup found no authorization servers",
        ));
    }
    if !authorization_servers.is_empty() && authorization.metadata.is_none() {
        attempts.push(format!(
            "authorization server metadata fetch failed for: {}",
            redacted_origin_list(&authorization_servers)
        ));
    }
    DiscoveryReport {
        outcome: combine(authorization.metadata, scopes, failure_statuses),
        attempts,
    }
}
