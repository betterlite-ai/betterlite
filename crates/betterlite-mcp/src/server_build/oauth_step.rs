use betterlite_db::managed_resource_rows::mcp::{McpOAuth2Flow, McpServerTable};
use betterlite_db::mcp_stored_contracts::{MCPAuth, MCPTokenEndpointAuthMethod};

mod fetch;
mod request;
mod resolve;

use super::ServerBuilder;
use super::credentials::ResolvedCredentials;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::server_oauth::basics::{has_discovery_source, obo_needs_discovery};
use crate::server_oauth::warnings::{NeededEndpoint, discovery_leaves_unresolved};
use crate::server_record::{MCPOAuthMetadata, OAuth2Flow, ServerFlag};

use fetch::{fetch_table_metadata, restrict_for_table, warn_table_unresolved};
use request::table_oauth_request;
use resolve::resolve_oauth_fields;

pub struct TableOAuthRequest<'table> {
    pub server_id: &'table str,
    pub server_ref: &'table str,
    pub server_url: Option<&'table str>,
    pub auth_type: Option<MCPAuth>,
    pub oauth2_flow: Option<McpOAuth2Flow>,
    pub dcr_bridge: Option<bool>,
    pub manual_issuer: Option<&'table str>,
    pub manual_authorization_url: Option<&'table str>,
    pub manual_token_url: Option<&'table str>,
    pub manual_registration_url: Option<&'table str>,
    pub is_discovery_auth_type: bool,
    pub use_issuer_anchor: bool,
    pub scopes: Option<&'table [String]>,
    pub token_exchange_endpoint: Option<String>,
}

pub struct OAuthResolution {
    pub oauth2_flow: Option<OAuth2Flow>,
    pub scopes: Option<Vec<String>>,
    pub configured_scopes: Option<Vec<String>>,
    pub issuer: Option<String>,
    pub issuer_is_anchored: bool,
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub registration_url: Option<String>,
    pub configured_authorization_url: Option<String>,
    pub configured_token_url: Option<String>,
    pub configured_registration_url: Option<String>,
    pub token_endpoint_auth_method: Option<MCPTokenEndpointAuthMethod>,
}

struct TableNeeds {
    needs_authorization_url: ServerFlag,
    needs_token_url: ServerFlag,
    needs_discovery: ServerFlag,
    deferred: ServerFlag,
    warn_on_empty: ServerFlag,
}

impl<D, F, S, R> ServerBuilder<'_, D, F, S, R>
where
    D: Sync,
    F: MetadataFetcher,
    S: Sync,
    R: Sync,
{
    #[must_use]
    pub async fn resolve_table_oauth_metadata(
        &self,
        request: &TableOAuthRequest<'_>,
    ) -> (Option<MCPOAuthMetadata>, bool) {
        let needs = self.table_needs(request);
        let metadata = if needs.needs_discovery && !needs.deferred {
            fetch_table_metadata(
                self.fetcher,
                self.settings.fetch,
                request,
                needs.warn_on_empty,
            )
            .await
        } else {
            None
        };
        if request.use_issuer_anchor {
            return (metadata, needs.deferred);
        }
        let gated = restrict_for_table(metadata, request);
        warn_table_unresolved(request, &needs, gated.as_ref());
        (gated, needs.deferred)
    }

    pub(super) async fn table_oauth(
        &self,
        row: &McpServerTable,
        credentials: &ResolvedCredentials,
        name: &str,
    ) -> OAuthResolution {
        let request = table_oauth_request(row, credentials, name);
        let (metadata, _deferred) = self.resolve_table_oauth_metadata(&request).await;
        resolve_oauth_fields(row, credentials, &request, metadata.as_ref())
    }

    fn table_needs(&self, request: &TableOAuthRequest<'_>) -> TableNeeds {
        let obo = obo_needs_discovery(
            request.auth_type,
            request.token_exchange_endpoint.as_deref(),
            request.manual_token_url,
        );
        let needs_authorization_url = request.is_discovery_auth_type
            && request.oauth2_flow != Some(McpOAuth2Flow::ClientCredentials);
        let needs_token_url = request.is_discovery_auth_type || obo;
        let warn_on_empty = discovery_leaves_unresolved(
            NeededEndpoint {
                needed: needs_authorization_url,
                manual: request.manual_authorization_url,
            },
            NeededEndpoint {
                needed: needs_token_url,
                manual: request.manual_token_url,
            },
        );
        let has_all = request.manual_authorization_url.is_some()
            && request.manual_token_url.is_some()
            && request.scopes.is_some();
        let needs_discovery = has_discovery_source(request.server_url, request.use_issuer_anchor)
            && ((request.is_discovery_auth_type && !has_all) || obo);
        let deferred = needs_discovery && !self.settings.on_startup;
        TableNeeds {
            needs_authorization_url,
            needs_token_url,
            needs_discovery,
            deferred,
            warn_on_empty,
        }
    }
}
