use super::authorization::fetch_single_issuer_metadata;
use super::fetch::MetadataFetcher;
use super::outcome::{DiscoveryOutcome, DiscoveryReport, DiscoveryStatus};
use super::redaction::{UNPARSABLE_URL, sanitized_error_text};
use super::server_document::discover_from_server_document;
use super::settings::DiscoverySettings;
use super::status_error::discover_after_status_error;
use crate::oauth_resource::resource_identity::redact_mcp_resource_url;

#[derive(Clone, Copy)]
pub struct DiscoveryRequest<'url> {
    pub server_url: &'url str,
    pub allow_origin_fallback: bool,
    pub warn_when_no_metadata: bool,
}

pub async fn discover_metadata<F>(
    fetcher: &F,
    settings: DiscoverySettings,
    request: DiscoveryRequest<'_>,
) -> DiscoveryOutcome
where
    F: MetadataFetcher,
{
    let report = discover_recording_attempts(fetcher, settings, request).await;
    if request.warn_when_no_metadata {
        warn_when_empty(request.server_url, &report);
    }
    report.outcome
}

pub async fn discover_recording_attempts<F>(
    fetcher: &F,
    settings: DiscoverySettings,
    request: DiscoveryRequest<'_>,
) -> DiscoveryReport
where
    F: MetadataFetcher,
{
    let origin = redact_mcp_resource_url(Some(request.server_url))
        .unwrap_or_else(|| UNPARSABLE_URL.to_owned());
    let response = match fetcher
        .get_json(request.server_url, &[], settings.metadata_timeout)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let message = sanitized_error_text(&error.message);
            return DiscoveryReport {
                outcome: DiscoveryOutcome::Failed(vec![DiscoveryStatus::Transport(
                    message.clone(),
                )]),
                attempts: vec![format!("GET {origin}: transport error: {message}")],
            };
        }
    };
    if !response.status.is_success() {
        return discover_after_status_error(fetcher, settings, request, &response, &origin).await;
    }
    discover_from_server_document(fetcher, settings, request, &response, &origin).await
}

pub async fn fetch_issuer_anchored_metadata<F>(
    fetcher: &F,
    issuer: &str,
    server_url: Option<&str>,
    settings: DiscoverySettings,
) -> DiscoveryOutcome
where
    F: MetadataFetcher,
{
    let lookup = fetch_single_issuer_metadata(fetcher, issuer, Some(issuer), settings).await;
    let Some(mut metadata) = lookup.metadata else {
        let origin =
            redact_mcp_resource_url(Some(issuer)).unwrap_or_else(|| UNPARSABLE_URL.to_owned());
        tracing::warn!(
            "MCP OAuth issuer-anchored discovery for issuer {origin} yielded no metadata whose issuer matched (RFC 8414 section 3.3); OAuth endpoints stay unresolved until a rebuild succeeds"
        );
        return DiscoveryOutcome::Failed(lookup.statuses);
    };
    metadata.scopes = resource_scopes(fetcher, settings, server_url).await;
    DiscoveryOutcome::Found(metadata)
}

async fn resource_scopes<F>(
    fetcher: &F,
    settings: DiscoverySettings,
    server_url: Option<&str>,
) -> Option<Vec<String>>
where
    F: MetadataFetcher,
{
    let url = server_url?;
    let request = DiscoveryRequest {
        server_url: url,
        allow_origin_fallback: false,
        warn_when_no_metadata: false,
    };
    match discover_metadata(fetcher, settings, request).await {
        DiscoveryOutcome::Found(metadata) => metadata.scopes,
        DiscoveryOutcome::Failed(_) => None,
    }
}

fn warn_when_empty(server_url: &str, report: &DiscoveryReport) {
    if matches!(&report.outcome, DiscoveryOutcome::Found(_)) {
        return;
    }
    let origin =
        redact_mcp_resource_url(Some(server_url)).unwrap_or_else(|| UNPARSABLE_URL.to_owned());
    let attempts = if report.attempts.is_empty() {
        String::from("none recorded")
    } else {
        report.attempts.join("; ")
    };
    tracing::warn!(
        "MCP OAuth endpoint discovery against {origin} found no authorization server metadata. Attempts: {attempts}. The MCP server url may be misconfigured, or the upstream may not support OAuth discovery (RFC 9728 / RFC 8414)"
    );
}
