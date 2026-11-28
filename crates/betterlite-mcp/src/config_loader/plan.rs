use betterlite_db::mcp_stored_contracts::MCPAuth;
use betterlite_models::decisions::Json;

use super::config::McpServerConfig;
use super::record::ResolvedOAuth;
use crate::oauth_discovery::fetch::MetadataFetcher;
use crate::oauth_discovery::flow::{
    DiscoveryRequest, discover_metadata, fetch_issuer_anchored_metadata,
};
use crate::oauth_discovery::outcome::DiscoveryOutcome;
use crate::oauth_discovery::settings::DiscoverySettings;
use crate::server_oauth::basics::{
    UPSTREAM_DISCOVERY_AUTH_TYPES, blank_to_none, has_discovery_source, obo_needs_discovery,
    uses_issuer_anchor,
};
use crate::server_oauth::flow::explicit_oauth2_flow;
use crate::server_oauth::issuer::{ManualEndpoints, endpoints_yield_to_issuer};
use crate::server_oauth::trust::restrict_to_corroborated;
use crate::server_oauth::warnings::{NeededEndpoint, discovery_leaves_unresolved};
use crate::server_record::{MCPOAuthMetadata, OAuth2Flow, ServerFlag};

pub(super) struct OAuthPlan {
    pub auth_type: Option<MCPAuth>,
    pub flow: Option<OAuth2Flow>,
    pub manual_issuer: Option<String>,
    pub endpoints: EndpointPlan,
    pub is_discovery_auth_type: ServerFlag,
    pub use_issuer_anchor: ServerFlag,
    pub should_discover: ServerFlag,
    pub deferred: ServerFlag,
    pub needs_authorization_url: ServerFlag,
    pub needs_token_url: ServerFlag,
    pub warn_on_empty: ServerFlag,
}

pub(super) struct EndpointPlan {
    pub configured_authorization: Option<String>,
    pub configured_token: Option<String>,
    pub configured_registration: Option<String>,
    pub manual_authorization: Option<String>,
    pub manual_token: Option<String>,
    pub manual_registration: Option<String>,
}

struct ResolvedEndpoints {
    authorization: Option<String>,
    token: Option<String>,
    registration: Option<String>,
}

struct DiscoveryPlan {
    should_discover: ServerFlag,
    needs_authorization_url: ServerFlag,
    needs_token_url: ServerFlag,
    warn_on_empty: ServerFlag,
}

struct PlanInputs {
    auth_type: Option<MCPAuth>,
    raw_flow: Option<String>,
    manual_issuer: Option<String>,
    is_discovery_auth_type: ServerFlag,
    obo: ServerFlag,
}

pub(super) fn oauth_plan(
    config: &McpServerConfig,
    server_ref: &str,
    server_url: &str,
    on_startup: bool,
) -> OAuthPlan {
    let inputs = plan_inputs(config);
    let anchored = uses_issuer_anchor(
        inputs.manual_issuer.as_deref(),
        inputs.is_discovery_auth_type || inputs.obo,
    );
    let endpoints = configured_endpoints(
        config,
        inputs.manual_issuer.as_deref(),
        inputs.is_discovery_auth_type,
        server_ref,
    );
    let discovery = discovery_plan(server_url, anchored, &inputs, &endpoints);
    OAuthPlan {
        auth_type: inputs.auth_type,
        flow: explicit_oauth2_flow(inputs.raw_flow.as_deref()),
        manual_issuer: inputs.manual_issuer,
        endpoints,
        is_discovery_auth_type: inputs.is_discovery_auth_type,
        use_issuer_anchor: anchored,
        should_discover: discovery.should_discover,
        deferred: discovery.should_discover && !on_startup,
        needs_authorization_url: discovery.needs_authorization_url,
        needs_token_url: discovery.needs_token_url,
        warn_on_empty: discovery.warn_on_empty,
    }
}

fn plan_inputs(config: &McpServerConfig) -> PlanInputs {
    let auth_type = config.auth_type;
    let raw_flow = config.oauth2_flow.clone();
    let manual_issuer = blank_to_none(config.issuer.as_deref()).map(str::to_owned);
    let is_discovery_auth_type =
        auth_type.is_some_and(|auth| UPSTREAM_DISCOVERY_AUTH_TYPES.contains(&auth));
    let obo = obo_needs_discovery(
        auth_type,
        config.token_exchange_endpoint.as_deref(),
        config.token_url.as_deref(),
    );
    PlanInputs {
        auth_type,
        raw_flow,
        manual_issuer,
        is_discovery_auth_type,
        obo,
    }
}

fn discovery_plan(
    server_url: &str,
    anchored: ServerFlag,
    inputs: &PlanInputs,
    endpoints: &EndpointPlan,
) -> DiscoveryPlan {
    let should_discover = has_discovery_source(Some(server_url), anchored)
        && (inputs.is_discovery_auth_type || inputs.obo);
    let needs_authorization_url =
        inputs.is_discovery_auth_type && inputs.raw_flow.as_deref() != Some("client_credentials");
    let needs_token_url = inputs.is_discovery_auth_type || inputs.obo;
    let warn_on_empty = discovery_leaves_unresolved(
        NeededEndpoint {
            needed: needs_authorization_url,
            manual: endpoints.manual_authorization.as_deref(),
        },
        NeededEndpoint {
            needed: needs_token_url,
            manual: endpoints.manual_token.as_deref(),
        },
    );
    DiscoveryPlan {
        should_discover,
        needs_authorization_url,
        needs_token_url,
        warn_on_empty,
    }
}

fn configured_endpoints(
    config: &McpServerConfig,
    manual_issuer: Option<&str>,
    is_discovery_auth_type: bool,
    server_ref: &str,
) -> EndpointPlan {
    let configured_authorization =
        blank_to_none(config.authorization_url.as_deref()).map(str::to_owned);
    let configured_token = blank_to_none(config.token_url.as_deref()).map(str::to_owned);
    let configured_registration =
        blank_to_none(config.registration_url.as_deref()).map(str::to_owned);
    let manual = endpoints_yield_to_issuer(
        manual_issuer,
        is_discovery_auth_type,
        ManualEndpoints {
            authorization_url: configured_authorization.as_deref(),
            token_url: configured_token.as_deref(),
            registration_url: configured_registration.as_deref(),
        },
        server_ref,
    );
    let manual_authorization = manual.authorization_url.map(str::to_owned);
    let manual_token = manual.token_url.map(str::to_owned);
    let manual_registration = manual.registration_url.map(str::to_owned);
    EndpointPlan {
        configured_authorization,
        configured_token,
        configured_registration,
        manual_authorization,
        manual_token,
        manual_registration,
    }
}

pub(super) async fn fetch_metadata<F>(
    fetcher: &F,
    plan: &OAuthPlan,
    settings: DiscoverySettings,
    server_url: &str,
) -> Option<MCPOAuthMetadata>
where
    F: MetadataFetcher,
{
    if !plan.should_discover || plan.deferred {
        return None;
    }
    if plan.use_issuer_anchor {
        let issuer = plan.manual_issuer.as_deref()?;
        let resource = (!server_url.is_empty()).then_some(server_url);
        let outcome = fetch_issuer_anchored_metadata(fetcher, issuer, resource, settings).await;
        return found_metadata(outcome);
    }
    let request = DiscoveryRequest {
        server_url,
        allow_origin_fallback: plan.is_discovery_auth_type,
        warn_when_no_metadata: plan.warn_on_empty,
    };
    found_metadata(discover_metadata(fetcher, settings, request).await)
}

fn found_metadata(outcome: DiscoveryOutcome) -> Option<MCPOAuthMetadata> {
    match outcome {
        DiscoveryOutcome::Found(metadata) => Some(metadata),
        DiscoveryOutcome::Failed(_) => None,
    }
}

pub(super) fn gate_metadata(
    plan: &OAuthPlan,
    server_ref: &str,
    dcr_bridge: Option<&Json>,
    metadata: Option<MCPOAuthMetadata>,
) -> Option<MCPOAuthMetadata> {
    if plan.use_issuer_anchor {
        return metadata;
    }
    if plan.is_discovery_auth_type {
        return restrict_to_corroborated(
            metadata,
            plan.endpoints.manual_authorization.as_deref(),
            server_ref,
            matches!(dcr_bridge, Some(Json::Bool(true))),
        );
    }
    metadata
}

pub(super) fn resolved_oauth(
    plan: &OAuthPlan,
    scopes: Option<Vec<String>>,
    configured_scopes: Option<Vec<String>>,
    metadata: Option<MCPOAuthMetadata>,
) -> ResolvedOAuth {
    let endpoints = resolved_endpoints(plan, metadata.as_ref());
    let issuer = plan.manual_issuer.clone().or_else(|| {
        let found = metadata.as_ref()?;
        if found.from_origin_fallback {
            return None;
        }
        found.discovered_issuer.clone()
    });
    ResolvedOAuth {
        flow: plan.flow,
        scopes,
        configured_scopes,
        issuer,
        issuer_is_anchored: plan.use_issuer_anchor,
        authorization_url: endpoints.authorization,
        token_url: endpoints.token,
        registration_url: endpoints.registration,
        configured_authorization_url: plan.endpoints.configured_authorization.clone(),
        configured_token_url: plan.endpoints.configured_token.clone(),
        configured_registration_url: plan.endpoints.configured_registration.clone(),
        metadata,
    }
}

fn resolved_endpoints(plan: &OAuthPlan, metadata: Option<&MCPOAuthMetadata>) -> ResolvedEndpoints {
    let manual = &plan.endpoints;
    ResolvedEndpoints {
        authorization: manual
            .manual_authorization
            .clone()
            .or_else(|| metadata?.authorization_url.clone()),
        token: manual
            .manual_token
            .clone()
            .or_else(|| metadata?.token_url.clone()),
        registration: manual
            .manual_registration
            .clone()
            .or_else(|| metadata?.registration_url.clone()),
    }
}
