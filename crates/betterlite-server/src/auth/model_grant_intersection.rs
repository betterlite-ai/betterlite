use std::fmt;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::as_proxy_exception;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_auth::model_grant_intersection::enforced::{
    EnforcedRequest, enforced_model_allowlists,
};
use betterlite_auth::model_grant_intersection::sources::{GrantSources, ServingGroups};
use betterlite_auth::model_grant_intersection::stamp::{StampRequest, stamp_matched_groups};
use betterlite_auth::model_grant_intersection::token::GrantToken;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::AuthContext;
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

// The key, the earlier-stage rows, the requested models and the reads the walk needs.
#[derive(Clone, Default)]
pub struct GrantRule {
    pub key: Option<UserAPIKeyAuth>,
    pub team: Option<LiteLLMTeamTableCachedObj>,
    pub project: Option<CachedProject>,
    pub membership: Option<LiteLLMTeamMembership>,
    pub membership_loaded: bool,
    pub models: Option<Vec<String>>,
    pub sources: Option<Arc<dyn GrantSources>>,
    pub groups: Option<Arc<dyn ServingGroups>>,
    pub sink: Option<Arc<dyn GrantSink>>,
}

// the collaborators stay out of the line, since they carry the cache, the database handles and the router
impl fmt::Debug for GrantRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrantRule")
            .field("models", &self.models)
            .field("membership_loaded", &self.membership_loaded)
            .finish_non_exhaustive()
    }
}

// The request-scoped carrier the assembled allowlists and the stamped groups reach; the state owner wires it.
pub trait GrantSink: Send + Sync {
    fn record(&self, allowlists: &[Vec<String>], matched: &[String]);
}

#[derive(Clone, Debug)]
pub struct GrantStageInput {
    pub context: AuthContext,
    pub rule: Arc<GrantRule>,
    pub database: bool,
    pub now: f64,
}

pub(crate) fn rule_for_request(request: &Request<Body>) -> Arc<GrantRule> {
    request
        .extensions()
        .get::<Arc<GrantRule>>()
        .map_or_else(|| Arc::new(GrantRule::default()), Arc::clone)
}

// the lookup is asynchronous, so the stage keeps its own input and the caller awaits the prefetch
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<GrantStageInput, AuthenticationFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let database = request
        .extensions()
        .get::<Arc<ServerState>>()
        .is_some_and(|state| state.model_grant_intersection.database);
    Ok(GrantStageInput {
        context,
        rule: rule_for_request(request),
        database,
        now: now_seconds(),
    })
}

// A lookup failure stops the request; the attribution itself is best-effort and never fails it.
pub(crate) async fn prefetch(stage: &GrantStageInput) -> Result<(), Box<ProxyException>> {
    let rule = &stage.rule;
    let Some(key) = rule.key.as_ref() else {
        return Ok(());
    };
    let sources = match (stage.database, rule.sources.as_deref()) {
        (true, Some(sources)) => Some(sources),
        (true, None) => return Ok(()),
        (false, _) => None,
    };
    let token = GrantToken::of(key);
    let allowlists = enforced_model_allowlists(EnforcedRequest {
        token: &token,
        sources,
        now: stage.now,
    })
    .await
    .map_err(|error| {
        let failure = error.as_auth_failure();
        Box::new(as_proxy_exception(&failure).exception().clone())
    })?;
    let matched = match sources {
        Some(sources) => stamped(key, rule, sources).await,
        None => Vec::new(),
    };
    if let Some(sink) = rule.sink.as_deref() {
        sink.record(&allowlists, &matched);
    }
    Ok(())
}

async fn stamped(
    key: &UserAPIKeyAuth,
    rule: &GrantRule,
    sources: &dyn GrantSources,
) -> Vec<String> {
    let mut stamped = key.clone();
    stamp_matched_groups(StampRequest {
        models: rule.models.as_deref(),
        auth: Some(&mut stamped),
        team: rule.team.as_ref(),
        project: rule.project.as_ref(),
        membership: rule.membership.clone(),
        membership_loaded: rule.membership_loaded,
        groups: rule.groups.as_deref(),
        sources: Some(sources),
    })
    .await
}

// get_project_object stamps the cached project with the cache's clock; the stage reads the wall clock once
fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0_f64, |elapsed| elapsed.as_secs_f64())
}
