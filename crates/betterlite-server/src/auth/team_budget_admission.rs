use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use betterlite_auth::auth_end_users::defaults::MemberBudgetError;
use betterlite_auth::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use betterlite_auth::auth_key_budgets::alerts::BudgetAlertSink;
use betterlite_auth::auth_key_budgets::spend::{SpendReadError, SpendReader};
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::{
    ProviderInference, UnresolvedProviderInference,
};
use betterlite_auth::auth_model_patterns::view::ModelAccessView;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::current_spend::SpendQuery;
use betterlite_auth::membership_load::error::MembershipLoadError;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::team_budget_admission::alerts::TeamAlertSink;
use betterlite_auth::team_budget_admission::error::{TeamBudgetError, membership_failure};
use betterlite_auth::team_budget_admission::sources::{
    DefaultBudgetSource, Membership, MembershipSource,
};
use betterlite_auth::team_budget_admission::{TeamAdmission, TeamRequest, admit};
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_exception::ProxyException;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::auth::auth_identity::{AuthContext, StageFailure, StageInput};
use crate::auth::auth_model_patterns::max_recurse_depth_for_request;
use crate::auth::current_spend::spend_failure_exception;
use crate::error_body::AuthenticationFailure;

mod tests;

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;

// what the auth service resolved before this stage: the team row, the key, the request model and the collaborators
pub struct TeamBudgetRule {
    pub team: Option<LiteLLMTeamTable>,
    pub auth: Option<UserAPIKeyAuth>,
    pub user_email: Option<String>,
    pub model: RequestedModels,
    pub membership: Membership,
    pub view: Option<Arc<dyn ModelAccessView>>,
    pub provider: Option<Arc<dyn ProviderInference>>,
    pub global_model_aliases: BTreeMap<String, String>,
    pub reader: Option<Arc<dyn SpendReader>>,
    pub memberships: Option<Arc<dyn MembershipSource>>,
    pub defaults: Option<Arc<dyn DefaultBudgetSource>>,
    pub member_alerts: Option<Arc<dyn BudgetAlertSink>>,
    pub team_alerts: Option<Arc<dyn TeamAlertSink>>,
    pub durations: Option<Arc<dyn DurationParser + Send + Sync>>,
    pub clock: fn() -> DateTime<Utc>,
}

impl Default for TeamBudgetRule {
    fn default() -> Self {
        Self {
            team: None,
            auth: None,
            user_email: None,
            model: RequestedModels::None,
            membership: Membership::Unloaded,
            view: None,
            provider: None,
            global_model_aliases: BTreeMap::new(),
            reader: None,
            memberships: None,
            defaults: None,
            member_alerts: None,
            team_alerts: None,
            durations: None,
            clock: Utc::now,
        }
    }
}

impl fmt::Debug for TeamBudgetRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TeamBudgetRule")
            .field(
                "team_id",
                &self.team.as_ref().map(|team| &team.fields.base.team_id),
            )
            .field("has_auth", &self.auth.is_some())
            .field("model", &self.model)
            .field("has_reader", &self.reader.is_some())
            .field("has_memberships", &self.memberships.is_some())
            .field("has_defaults", &self.defaults.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct TeamBudgetStageInput {
    pub context: AuthContext,
    pub rule: Arc<TeamBudgetRule>,
    pub max_recurse_depth: i64,
}

// the authentication service inserts the rule once it resolved the team; without one the stage is inert
pub(crate) fn stage_input(
    request: &Request<Body>,
    authenticated: Option<&AuthContext>,
) -> Result<StageInput, StageFailure> {
    let context = authenticated
        .cloned()
        .ok_or(AuthenticationFailure::Missing)?;
    let rule = request
        .extensions()
        .get::<Arc<TeamBudgetRule>>()
        .map_or_else(|| Arc::new(TeamBudgetRule::default()), Arc::clone);
    Ok(StageInput::TeamBudgetAdmission(Box::new(
        TeamBudgetStageInput {
            context,
            rule,
            max_recurse_depth: max_recurse_depth_for_request(request),
        },
    )))
}

pub(crate) async fn prefetched(
    stage: Box<TeamBudgetStageInput>,
) -> Result<StageInput, StageFailure> {
    prefetch(&stage).await.map_err(StageFailure::TeamBudgets)?;
    Ok(StageInput::TeamBudgetAdmission(stage))
}

// the member scope, then every team budget check; a denial or a failed read is the stage's failure
pub(crate) async fn prefetch(stage: &TeamBudgetStageInput) -> Result<(), Box<ProxyException>> {
    let rule = &stage.rule;
    let provider: &dyn ProviderInference = rule.provider.as_deref().unwrap_or(&UNRESOLVED_PROVIDER);
    let models = ModelAccessContext {
        view: rule.view.as_deref(),
        global_model_aliases: &rule.global_model_aliases,
        compaction: None,
        provider,
        max_recurse_depth: stage.max_recurse_depth,
    };
    let admission = TeamAdmission {
        reader: rule.reader.as_deref().unwrap_or(&Unconfigured),
        memberships: rule.memberships.as_deref().unwrap_or(&Unconfigured),
        defaults: rule.defaults.as_deref().unwrap_or(&Unconfigured),
        member_alerts: rule.member_alerts.as_deref(),
        team_alerts: rule.team_alerts.as_deref(),
        durations: rule.durations.as_deref().unwrap_or(&Unconfigured),
        models: &models,
        now: (rule.clock)(),
    };
    let request = TeamRequest {
        team: rule.team.as_ref(),
        token: rule.auth.as_ref(),
        user_email: rule.user_email.as_deref(),
        model: &rule.model,
        membership: rule.membership.clone(),
    };
    admit(&admission, &request)
        .await
        .map(|_| ())
        .map_err(|error| Box::new(team_budget_exception(&error)))
}

// _as_proxy_exception: a budget denial keeps its status, the member denial is already a ProxyException, a membership outage is B1217's 503 and the rest are 401s
fn team_budget_exception(error: &TeamBudgetError) -> ProxyException {
    let failure = match error {
        TeamBudgetError::Denied(denial) => AuthFailure::BudgetExceeded {
            message: denial.message.clone(),
            status_code: Some(denial.status_code),
            llm_provider: String::new(),
        },
        TeamBudgetError::ModelDenied(denial) => return denial.as_proxy_exception().clone(),
        TeamBudgetError::Spend(failure) => return spend_failure_exception(failure),
        TeamBudgetError::Membership(error) => membership_failure(error),
        TeamBudgetError::DefaultBudget(_) | TeamBudgetError::RecurseDepth { .. } => {
            AuthFailure::Other {
                message: error.to_string(),
                param: None,
            }
        }
    };
    as_proxy_exception(&failure).exception().clone()
}

// a rule without a reader cannot verify spend (the 503), without a database cannot load a membership, and has no defaults or durations
struct Unconfigured;

#[async_trait]
impl SpendReader for Unconfigured {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        Err(SpendReadError::Unverifiable {
            counter_key: query.counter_key.to_owned(),
        })
    }
}

#[async_trait]
impl MembershipSource for Unconfigured {
    async fn team_membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        Err(MembershipLoadError::NoDatabase)
    }
}

#[async_trait]
impl DefaultBudgetSource for Unconfigured {
    async fn team_member_default(
        &self,
        _budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        Ok(None)
    }
}

impl DurationParser for Unconfigured {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}
