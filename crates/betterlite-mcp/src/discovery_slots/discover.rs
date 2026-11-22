use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::oauth_discovery::flow::{
    DiscoveryRequest, discover_metadata, fetch_issuer_anchored_metadata,
};
use crate::oauth_discovery::outcome::{DiscoveryOutcome, DiscoveryStatus};
use crate::oauth_discovery::settings::DiscoverySettings as FetchSettings;
use crate::server_oauth::basics::{
    UPSTREAM_DISCOVERY_AUTH_TYPES, blank_to_none, obo_needs_discovery,
};
use crate::server_oauth::trust::restrict_to_corroborated;
use crate::server_oauth::warnings::{
    NeededEndpoint, UnresolvedCheck, discovery_leaves_unresolved, warn_unresolved,
};
use crate::server_record::{MCPOAuthMetadata, MCPServer, OAuth2Flow};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryAttempt {
    pub metadata: Option<MCPOAuthMetadata>,
    pub retryable: bool,
}

#[derive(Clone, Copy)]
pub(super) struct DiscoveryNeeds<'server> {
    pub(super) is_discovery_auth_type: bool,
    pub(super) authorization: NeededEndpoint<'server>,
    pub(super) token: NeededEndpoint<'server>,
}

pub async fn discover_server<F>(
    fetcher: &F,
    settings: FetchSettings,
    server: &MCPServer,
) -> DiscoveryAttempt
where
    F: MetadataFetcher,
{
    let needs = discovery_needs(server);
    let outcome = fetch_server_metadata(fetcher, settings, server, needs).await;
    let (found, retryable) = match outcome {
        DiscoveryOutcome::Found(metadata) => (Some(metadata), false),
        DiscoveryOutcome::Failed(statuses) => (None, statuses.iter().any(is_retryable)),
    };
    if server.fields.issuer_is_anchored {
        return DiscoveryAttempt {
            metadata: found,
            retryable,
        };
    }
    DiscoveryAttempt {
        metadata: gate_discovered(server, needs, found),
        retryable,
    }
}

#[must_use]
pub fn is_retryable(status: &DiscoveryStatus) -> bool {
    match status {
        DiscoveryStatus::Transport(_) => true,
        DiscoveryStatus::HttpError(code) => code.is_server_error(),
        DiscoveryStatus::NotFound
        | DiscoveryStatus::Invalid
        | DiscoveryStatus::IssuerMismatch
        | DiscoveryStatus::CrossOrigin => false,
    }
}

pub(super) fn discovery_needs(server: &MCPServer) -> DiscoveryNeeds<'_> {
    let fields = &server.fields;
    let manual_token_url = blank_to_none(fields.token_url.as_deref());
    let is_discovery_auth_type = fields
        .auth_type
        .is_some_and(|auth| UPSTREAM_DISCOVERY_AUTH_TYPES.contains(&auth));
    let obo_needs_token = obo_needs_discovery(
        fields.auth_type,
        fields.token_exchange_endpoint.as_deref(),
        manual_token_url,
    );
    DiscoveryNeeds {
        is_discovery_auth_type,
        authorization: NeededEndpoint {
            needed: is_discovery_auth_type
                && fields.oauth2_flow != Some(OAuth2Flow::ClientCredentials),
            manual: blank_to_none(fields.authorization_url.as_deref()),
        },
        token: NeededEndpoint {
            needed: is_discovery_auth_type || obo_needs_token,
            manual: manual_token_url,
        },
    }
}

async fn fetch_server_metadata<F>(
    fetcher: &F,
    settings: FetchSettings,
    server: &MCPServer,
    needs: DiscoveryNeeds<'_>,
) -> DiscoveryOutcome
where
    F: MetadataFetcher,
{
    let fields = &server.fields;
    let anchored_issuer =
        blank_to_none(fields.issuer.as_deref()).filter(|_| fields.issuer_is_anchored);
    if let Some(issuer) = anchored_issuer {
        return fetch_issuer_anchored_metadata(fetcher, issuer, fields.url.as_deref(), settings)
            .await;
    }
    let request = DiscoveryRequest {
        server_url: fields.url.as_deref().unwrap_or_default(),
        allow_origin_fallback: needs.is_discovery_auth_type,
        warn_when_no_metadata: discovery_leaves_unresolved(needs.authorization, needs.token),
    };
    discover_metadata(fetcher, settings, request).await
}

fn gate_discovered(
    server: &MCPServer,
    needs: DiscoveryNeeds<'_>,
    found: Option<MCPOAuthMetadata>,
) -> Option<MCPOAuthMetadata> {
    let fields = &server.fields;
    let metadata = if needs.is_discovery_auth_type {
        restrict_to_corroborated(
            found,
            needs.authorization.manual,
            &fields.server_id,
            server.is_dcr_bridge(),
        )
    } else {
        found
    };
    warn_unresolved(UnresolvedCheck {
        server_ref: fields
            .alias
            .as_deref()
            .or(fields.server_name.as_deref())
            .unwrap_or(&fields.server_id),
        server_url: fields.url.as_deref(),
        discovery_attempted: true,
        discovery_deferred: false,
        issuer_anchored: false,
        metadata: metadata.as_ref(),
        needs_authorization_url: needs.authorization.needed,
        needs_token_url: needs.token.needed,
        manual_authorization_url: needs.authorization.manual,
        manual_token_url: needs.token.manual,
    });
    metadata
}
