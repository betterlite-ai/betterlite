use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_model_grants::access_source::AgentAccessSource;
use betterlite_auth::auth_model_grants::caller::check_agent_caller;
use betterlite_auth::auth_model_grants::ceiling::check_agent_ceiling;
use betterlite_auth::auth_model_grants::error::AgentModelGrantsError;
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::{
    ProviderInference, UnresolvedProviderInference,
};
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::auth_org_lookup::error::proxy_exception as org_lookup_exception;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::auth_model_patterns::{ModelAccessRejection, max_recurse_depth_for_request};
use crate::auth::team_budget_admission::{TeamBudgetRule, team_budget_exception};
use crate::error_body::AuthenticationFailure;
use crate::state::ServerState;

mod tests;

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

// what the auth service resolved before this stage: the caller's key, the requested model and the entity stores the caller checks read
pub struct AgentModelGrantsRule {
    pub key: Option<UserAPIKeyAuth>,
    pub model: RequestedModels,
    pub view: Option<Arc<dyn ModelAccessView>>,
    pub provider: Option<Arc<dyn ProviderInference>>,
    pub global_model_aliases: BTreeMap<String, String>,
    pub member_checks: Arc<TeamBudgetRule>,
}

impl fmt::Debug for AgentModelGrantsRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentModelGrantsRule")
            .field("has_key", &self.key.is_some())
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct AgentModelGrantsStageInput {
    pub context: AuthContext,
    pub rule: Option<Arc<AgentModelGrantsRule>>,
    pub source: Option<Arc<dyn AgentAccessSource>>,
    pub max_recurse_depth: i64,
}

impl fmt::Debug for AgentModelGrantsStageInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentModelGrantsStageInput")
            .field("rule", &self.rule)
            .field("has_source", &self.source.is_some())
            .field("max_recurse_depth", &self.max_recurse_depth)
            .finish_non_exhaustive()
    }
}

// the authentication service inserts the rule once it resolved the caller; without one the stage is inert
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let rule = request
        .extensions()
        .get::<Arc<AgentModelGrantsRule>>()
        .cloned();
    let source = request
        .extensions()
        .get::<Arc<ServerState>>()
        .and_then(|state| state.auth_model_grants.source.clone());
    Ok(StageInput::AuthModelGrants(Box::new(
        AgentModelGrantsStageInput {
            context,
            rule,
            source,
            max_recurse_depth: max_recurse_depth_for_request(request),
        },
    )))
}

pub(crate) async fn prefetched(
    stage: Box<AgentModelGrantsStageInput>,
) -> Result<StageInput, StageFailure> {
    prefetch(&stage).await.map_err(stage_failure)?;
    Ok(StageInput::AuthModelGrants(stage))
}

// the agent's own ceiling first, then what the invoking caller's key and team may reach
pub(crate) async fn prefetch(
    stage: &AgentModelGrantsStageInput,
) -> Result<(), AgentModelGrantsError> {
    let Some(rule) = stage.rule.as_deref() else {
        return Ok(());
    };
    let token = rule
        .key
        .as_ref()
        .or(stage.context.principal.source_key.0.as_ref());
    let context = ModelAccessContext {
        view: rule.view.as_deref(),
        global_model_aliases: &rule.global_model_aliases,
        compaction: None,
        provider: rule.provider.as_deref().unwrap_or(&UNRESOLVED_PROVIDER),
        max_recurse_depth: stage.max_recurse_depth,
    };
    let source = stage.source.as_deref();
    check_agent_ceiling(&rule.model, token, source, &context).await?;
    let admission = rule.member_checks.admission(&context);
    check_agent_caller(&rule.model, token, source, &admission, &context).await
}

// the agent and caller denials keep B747's response fields; a lookup failure is an authentication error
fn stage_failure(error: AgentModelGrantsError) -> StageFailure {
    match error {
        AgentModelGrantsError::Denied(denial) => {
            StageFailure::ModelAccess(Box::new(ModelAccessRejection::Denied(denial)))
        }
        AgentModelGrantsError::RecurseDepth { model } => {
            StageFailure::ModelAccess(Box::new(ModelAccessRejection::RecurseDepth { model }))
        }
        AgentModelGrantsError::Entity(entity) => match *entity {
            EntityModelAccessError::Denied(denial) => {
                StageFailure::ModelAccess(Box::new(ModelAccessRejection::Denied(denial)))
            }
            EntityModelAccessError::RecurseDepth { model } => {
                StageFailure::ModelAccess(Box::new(ModelAccessRejection::RecurseDepth { model }))
            }
            EntityModelAccessError::Member(budget) => {
                StageFailure::ModelGrants(Box::new(team_budget_exception(&budget)))
            }
            EntityModelAccessError::Project(lookup) => {
                StageFailure::ModelGrants(Box::new(org_lookup_exception(&lookup)))
            }
            EntityModelAccessError::TeamRow(row) => {
                tracing::debug!(error = %row, "the caller team snapshot did not build");
                StageFailure::ModelGrants(Box::new(lookup_exception(&row.to_string())))
            }
        },
        AgentModelGrantsError::Ceiling(groups) => {
            StageFailure::ModelGrants(Box::new(lookup_exception(&groups.to_string())))
        }
        AgentModelGrantsError::Caller(caller) => {
            StageFailure::ModelGrants(Box::new(lookup_exception(&caller.to_string())))
        }
        AgentModelGrantsError::Unavailable => StageFailure::ModelGrants(Box::new(
            lookup_exception(&AgentModelGrantsError::Unavailable.to_string()),
        )),
    }
}

fn lookup_exception(message: &str) -> ProxyException {
    let failure = AuthFailure::Other {
        message: message.to_owned(),
        param: None,
    };
    as_proxy_exception(&failure).exception().clone()
}
