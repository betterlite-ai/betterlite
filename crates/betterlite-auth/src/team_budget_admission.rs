pub mod alerts;
pub mod error;
pub mod hard;
pub mod member;
pub mod member_scope;
pub mod soft;
pub mod sources;
mod tests;
pub mod windows;

use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, Utc};

use crate::auth_key_budgets::alerts::BudgetAlertSink;
use crate::auth_key_budgets::spend::SpendReader;
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_request_models::RequestedModels;
use crate::budget_primitives::window_start::DurationParser;
use crate::resolved_key::auth::UserAPIKeyAuth;

use self::alerts::TeamAlertSink;
use self::error::TeamBudgetError;
use self::soft::SoftOutcome;
use self::sources::{DefaultBudgetSource, Membership, MembershipSource, resolve_membership};

// the collaborators the proxy wires: the cross-pod reader, both loads, both alert sinks, the model view and the clock
pub struct TeamAdmission<'deps> {
    pub reader: &'deps dyn SpendReader,
    pub memberships: &'deps dyn MembershipSource,
    pub defaults: &'deps dyn DefaultBudgetSource,
    pub member_alerts: Option<&'deps dyn BudgetAlertSink>,
    pub team_alerts: Option<&'deps dyn TeamAlertSink>,
    pub durations: &'deps (dyn DurationParser + Sync),
    pub models: &'deps ModelAccessContext<'deps>,
    pub now: DateTime<Utc>,
}

// what the auth service resolved for the request: the team row, the key, its user's email and the requested model
pub struct TeamRequest<'request> {
    pub team: Option<&'request LiteLLMTeamTable>,
    pub token: Option<&'request UserAPIKeyAuth>,
    pub user_email: Option<&'request str>,
    pub model: &'request RequestedModels,
    pub membership: Membership,
}

// common_checks' order: the member model scope first, then every budget check settles and the first error in scope order wins
pub async fn admit(
    admission: &TeamAdmission<'_>,
    request: &TeamRequest<'_>,
) -> Result<SoftOutcome, TeamBudgetError> {
    let loaded = TeamRequest {
        team: request.team,
        token: request.token,
        user_email: request.user_email,
        model: request.model,
        membership: shared_membership(admission, request).await?,
    };
    member_scope::check_team_member_model_access(admission, &loaded).await?;
    let hard = hard::team_max_budget_check(admission, &loaded).await;
    let windows = windows::team_multi_budget_check(admission, loaded.team).await;
    let soft = soft::team_soft_budget_check(loaded.team, loaded.token, admission.team_alerts);
    let member = member::check_team_member_budget(admission, &loaded).await;
    hard.and(windows).and(member).map(|()| soft)
}

// the scope and budget checks share one load, made only when the request has a team and a token user
async fn shared_membership(
    admission: &TeamAdmission<'_>,
    request: &TeamRequest<'_>,
) -> Result<Membership, TeamBudgetError> {
    let user_id = request
        .token
        .and_then(|token| token.view.token.user_id.as_deref());
    let (Some(team), Some(user)) = (request.team, user_id) else {
        return Ok(request.membership.clone());
    };
    let membership = resolve_membership(
        admission.memberships,
        &request.membership,
        user,
        &team.fields.base.team_id,
    )
    .await?;
    Ok(Membership::Loaded(membership))
}
