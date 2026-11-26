use betterlite_auth::auth_context_sources::claude::{
    AliasMap, ClaudeNormalization, ClaudeNormalizationInput, normalize_claude_model,
};
use betterlite_auth::auth_context_sources::context::{ModelContext, context_failure};
use betterlite_auth::auth_context_sources::router_alias::{
    RouterAliasInput, RouterAliasState, resolve_router_alias,
};
use betterlite_auth::auth_request_models::catalog::Collaborators;
use betterlite_auth::auth_request_models::sources::{
    DispatchContext, is_pass_through_dispatch, is_provider_pass_through,
};
use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::rewrite::{Markers, RewriteRule};
use crate::state::ServerState;

pub(super) struct Selection<'request> {
    pub route: &'request str,
    pub body: &'request JsonObject,
    pub headers: &'request JsonObject,
    pub query: &'request JsonObject,
    pub raw_available: bool,
}

pub(super) async fn select_model(
    state: &ServerState,
    rule: &RewriteRule,
    markers: &mut Markers,
    input: Selection<'_>,
) -> Result<Option<String>, ProxyException> {
    let slot = &state.auth_context_sources;
    let collaborators = Collaborators {
        catalog: Some(state.auth_request_models.resolver.catalog.as_ref()),
        decoders: state.auth_request_models.resolver.decoders.as_ref(),
    };
    let claude = normalize_claude_model(
        &ClaudeNormalizationInput {
            route: input.route,
            request_data: input.body,
            model_context: ModelContext {
                route: input.route,
                body: input.body,
                headers: Some(input.headers),
                query: Some(input.query),
                dispatch: DispatchContext {
                    endpoint_marker: rule.endpoint_marker,
                    path_params: &rule.path_params,
                },
                team_id: rule.team_id.as_deref(),
            },
            collaborators: &collaborators,
            router: Some(slot),
            team_id: rule.team_id.as_deref(),
            global_aliases: &state.config.sections.pre_call.model_alias_map,
            key_aliases: rule.key_aliases.as_ref().map(AliasMap::Json),
            team_model_aliases: rule.team_aliases.as_ref().map(AliasMap::Json),
            key_router_settings: rule.key_settings.as_ref(),
            normalized: markers.normalized,
        },
        rule,
    )
    .await
    .map_err(|error| context_failure(&error))?;
    Ok(apply_alias(rule, markers, input, claude).await)
}

async fn apply_alias(
    rule: &RewriteRule,
    markers: &mut Markers,
    input: Selection<'_>,
    claude: ClaudeNormalization,
) -> Option<String> {
    markers.normalized = claude.normalized;
    let requested = claude
        .source
        .as_deref()
        .or_else(|| match input.body.get("model") {
            Some(Json::Text(name)) => Some(name.as_str()),
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => None,
        });
    let dispatch = DispatchContext {
        endpoint_marker: rule.endpoint_marker,
        path_params: &rule.path_params,
    };
    let alias = resolve_router_alias(
        &RouterAliasInput {
            state: if markers.resolved {
                RouterAliasState::Resolved
            } else {
                RouterAliasState::Fresh
            },
            passthrough: is_pass_through_dispatch(&dispatch) || is_provider_pass_through(&dispatch),
            requested_model: requested,
            raw_body_available: input.raw_available,
            key_router_settings: rule.key_settings.as_ref(),
            team_id: rule.team_id.as_deref(),
            client_requested_model: markers.client_model.as_deref(),
        },
        rule,
    )
    .await;
    markers.resolved = alias.marker_set;
    if let Some(client) = alias.client_requested_model {
        markers.client_model = Some(client);
    }
    alias.target.or(claude.source)
}
