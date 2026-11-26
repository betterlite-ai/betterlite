use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::{
    ProviderInference, UnresolvedProviderInference,
};
use betterlite_auth::auth_model_patterns::user_model::can_user_call_model;
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::auth_org_lookup::error::proxy_exception as org_lookup_exception;
use betterlite_auth::auth_org_lookup::organization::OrgLookup;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_auth::entity_model_access::grant::{key_aliases, team_aliases};
use betterlite_auth::entity_model_access::org::can_org_access_model;
use betterlite_auth::entity_model_access::resolved::{ResolvedDeps, can_key_call_resolved_model};
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_error::proxy_exception::ProxyException;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::auth_model_patterns::{ModelAccessRejection, max_recurse_depth_for_request};
use crate::auth::team_budget_admission::team_budget_exception;
use crate::error_body::AuthenticationFailure;

mod tests;

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

// what the auth service resolved before this stage: the caller's key, user and organization rows, the requested model and the entity stores the owner checks read
pub struct EntityModelRule {
    pub key: Option<UserAPIKeyAuth>,
    pub user: Option<LiteLLMUserTable>,
    pub org: Option<OrganizationTable>,
    pub model: RequestedModels,
    pub view: Option<Arc<dyn ModelAccessView>>,
    pub provider: Option<Arc<dyn ProviderInference>>,
    pub global_model_aliases: BTreeMap<String, String>,
    pub teams: Option<Arc<TeamLookup>>,
    pub projects: Option<Arc<OrgLookup>>,
    pub now: fn() -> f64,
}

impl fmt::Debug for EntityModelRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntityModelRule")
            .field(
                "key_team_id",
                &self
                    .key
                    .as_ref()
                    .and_then(|token| token.view.token.team_id.as_ref()),
            )
            .field("has_user", &self.user.is_some())
            .field(
                "org_id",
                &self
                    .org
                    .as_ref()
                    .and_then(|org| org.organization_id.as_ref()),
            )
            .field("model", &self.model)
            .field("has_teams", &self.teams.is_some())
            .field("has_projects", &self.projects.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct EntityModelStageInput {
    pub context: AuthContext,
    pub rule: Option<Arc<EntityModelRule>>,
    pub max_recurse_depth: i64,
}

// the authentication service inserts the rule once it resolved the caller; without one the stage is inert
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let rule = request.extensions().get::<Arc<EntityModelRule>>().cloned();
    Ok(StageInput::EntityModelAccess(Box::new(
        EntityModelStageInput {
            context,
            rule,
            max_recurse_depth: max_recurse_depth_for_request(request),
        },
    )))
}

pub(crate) async fn prefetched(
    stage: Box<EntityModelStageInput>,
) -> Result<StageInput, StageFailure> {
    prefetch(&stage).await.map_err(stage_failure)?;
    Ok(StageInput::EntityModelAccess(stage))
}

// common_checks' user and organization grants, then the resolved key, team, member and project checks
pub(crate) async fn prefetch(stage: &EntityModelStageInput) -> Result<(), EntityModelAccessError> {
    let Some(rule) = stage.rule.as_deref() else {
        return Ok(());
    };
    let provider: &dyn ProviderInference = rule.provider.as_deref().unwrap_or(&UNRESOLVED_PROVIDER);
    let context = ModelAccessContext {
        view: rule.view.as_deref(),
        global_model_aliases: &rule.global_model_aliases,
        compaction: None,
        provider,
        max_recurse_depth: stage.max_recurse_depth,
    };
    let aliases = rule.key.as_ref().map_or_else(BTreeMap::new, team_aliases);
    let targets = rule.key.as_ref().map_or_else(BTreeMap::new, key_aliases);
    EntityModelAccessError::gate(can_user_call_model(
        &rule.model,
        rule.user.as_ref(),
        &targets,
        &context,
    ))?;
    can_org_access_model(&rule.model, rule.org.as_ref(), &aliases, &context)?;
    if let Some(token) = rule.key.as_ref() {
        // the pipeline's budget stage already ran the member scope, so this stage owns none
        let deps = ResolvedDeps {
            teams: rule.teams.as_deref(),
            projects: rule.projects.as_deref(),
            admission: None,
            now: (rule.now)(),
        };
        can_key_call_resolved_model(&rule.model, token, &context, &deps).await?;
    }
    Ok(())
}

// the denial keeps B747's response fields, the member and organization failures keep their translations
fn stage_failure(error: EntityModelAccessError) -> StageFailure {
    match error {
        EntityModelAccessError::Denied(denial) => {
            StageFailure::ModelAccess(Box::new(ModelAccessRejection::Denied(denial)))
        }
        EntityModelAccessError::RecurseDepth { model } => {
            StageFailure::ModelAccess(Box::new(ModelAccessRejection::RecurseDepth { model }))
        }
        EntityModelAccessError::Member(budget) => {
            StageFailure::EntityModelAccess(Box::new(team_budget_exception(&budget)))
        }
        EntityModelAccessError::Project(lookup) => {
            StageFailure::EntityModelAccess(Box::new(org_lookup_exception(&lookup)))
        }
        EntityModelAccessError::TeamRow(row) => {
            tracing::debug!(error = %row, "the fallback team snapshot did not build");
            let failure = AuthFailure::Other {
                message: row.to_string(),
                param: None,
            };
            let exception: ProxyException = as_proxy_exception(&failure).exception().clone();
            StageFailure::EntityModelAccess(Box::new(exception))
        }
    }
}
