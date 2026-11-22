use super::TableNeeds;
use super::TableOAuthRequest;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::oauth_discovery::flow::{
    DiscoveryRequest, discover_metadata, fetch_issuer_anchored_metadata,
};
use crate::oauth_discovery::outcome::DiscoveryOutcome;
use crate::oauth_discovery::settings::DiscoverySettings as FetchSettings;
use crate::server_oauth::trust::restrict_to_corroborated;
use crate::server_oauth::warnings::{UnresolvedCheck, warn_unresolved};
use crate::server_record::MCPOAuthMetadata;

pub(super) async fn fetch_table_metadata<F>(
    fetcher: &F,
    settings: FetchSettings,
    request: &TableOAuthRequest<'_>,
    warn_on_empty: bool,
) -> Option<MCPOAuthMetadata>
where
    F: MetadataFetcher,
{
    if request.use_issuer_anchor
        && let Some(issuer) = request.manual_issuer
    {
        let outcome =
            fetch_issuer_anchored_metadata(fetcher, issuer, request.server_url, settings).await;
        return found_metadata(outcome);
    }
    let url = request.server_url?;
    let discovery = DiscoveryRequest {
        server_url: url,
        allow_origin_fallback: request.is_discovery_auth_type,
        warn_when_no_metadata: warn_on_empty,
    };
    found_metadata(discover_metadata(fetcher, settings, discovery).await)
}

fn found_metadata(outcome: DiscoveryOutcome) -> Option<MCPOAuthMetadata> {
    match outcome {
        DiscoveryOutcome::Found(metadata) => Some(metadata),
        DiscoveryOutcome::Failed(_statuses) => None,
    }
}

pub(super) fn restrict_for_table(
    metadata: Option<MCPOAuthMetadata>,
    request: &TableOAuthRequest<'_>,
) -> Option<MCPOAuthMetadata> {
    if request.is_discovery_auth_type {
        return restrict_to_corroborated(
            metadata,
            request.manual_authorization_url,
            request.server_id,
            request.dcr_bridge == Some(true),
        );
    }
    metadata
}

pub(super) fn warn_table_unresolved(
    request: &TableOAuthRequest<'_>,
    needs: &TableNeeds,
    metadata: Option<&MCPOAuthMetadata>,
) {
    warn_unresolved(UnresolvedCheck {
        server_ref: request.server_ref,
        server_url: request.server_url,
        discovery_attempted: needs.needs_discovery,
        discovery_deferred: needs.deferred,
        issuer_anchored: false,
        metadata,
        needs_authorization_url: needs.needs_authorization_url,
        needs_token_url: needs.needs_token_url,
        manual_authorization_url: request.manual_authorization_url,
        manual_token_url: request.manual_token_url,
    });
}
