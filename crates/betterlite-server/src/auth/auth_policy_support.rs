use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, header};
use betterlite_auth::auth_policy_support::guardrails::guardrail_modification_check;
use betterlite_auth::auth_policy_support::tools::check_tools_allowlist;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::AuthContext;
use crate::auth::request_body_safety::request_object;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;
use crate::state::auth_policy_support::Slot;

mod tests;

// what the identity resolution read before this stage runs: the caller's key and its team
#[derive(Debug, Default)]
pub struct AuthPolicySupportRule {
    pub key: Option<UserAPIKeyAuth>,
    pub team: Option<LiteLLMTeamTable>,
}

#[derive(Clone, Debug)]
pub struct AuthPolicySupportStageInput {
    pub context: AuthContext,
    pub rule: Arc<AuthPolicySupportRule>,
    pub route: String,
    pub content_type: Option<String>,
    pub body: Option<Bytes>,
    pub slot: Option<Slot>,
}

// LiteLLM's common_checks tail: the guardrail opt-out check first, then the key/team tool allowlist
pub(crate) fn run_stage(
    stage: &AuthPolicySupportStageInput,
) -> Result<AuthContext, Box<ProxyException>> {
    let content_type = stage.content_type.as_deref().unwrap_or_default();
    let Some(body) = stage
        .body
        .as_ref()
        .and_then(|bytes| request_object(bytes, content_type))
    else {
        return Ok(stage.context.clone());
    };
    let team = stage.rule.team.as_ref();
    guardrail_modification_check(&body, team)
        .map_err(|error| Box::new(error.into_proxy_exception()))?;
    let Some(extractors) = stage
        .slot
        .as_ref()
        .and_then(|slot| slot.extractors.as_ref())
    else {
        return Ok(stage.context.clone());
    };
    check_tools_allowlist(
        &body,
        stage.rule.key.as_ref(),
        team,
        &stage.route,
        extractors,
    )
    .map_err(|error| Box::new(error.into_proxy_exception()))?;
    Ok(stage.context.clone())
}

pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
    body: Option<&Bytes>,
) -> Result<AuthPolicySupportStageInput, AuthenticationFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    Ok(AuthPolicySupportStageInput {
        context,
        rule: rule_for_request(request),
        route: request.uri().path().to_owned(),
        content_type,
        body: body.cloned(),
        slot: request
            .extensions()
            .get::<Arc<ServerState>>()
            .map(|state| state.auth_policy_support.clone()),
    })
}

fn rule_for_request(request: &Request<Body>) -> Arc<AuthPolicySupportRule> {
    request
        .extensions()
        .get::<Arc<AuthPolicySupportRule>>()
        .map_or_else(|| Arc::new(AuthPolicySupportRule::default()), Arc::clone)
}
