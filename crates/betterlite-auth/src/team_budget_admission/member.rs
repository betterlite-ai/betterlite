use betterlite_db::budget_permission_rows::{BudgetError, JsonData, LiteLLMBudgetTable};
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::auth_key_budgets::alerts::BudgetAlertSink;
use crate::auth_key_budgets::key_budget::{BUDGET_EXCEEDED_STATUS, Denial};
use crate::auth_key_budgets::money::Money;
use crate::auth_key_budgets::team_member::{TeamMemberAlert, team_member_alert_check};

use super::error::TeamBudgetError;
use super::sources::{member_budget_row, read_counter, resolve_membership};
use super::{TeamAdmission, TeamRequest};

pub const TEAM_MEMBER_BUDGET_ID_FIELD: &str = "team_member_budget_id";

// _check_team_member_budget: a member needs a team and a token user, and the counter is read before the finite test
pub async fn check_team_member_budget(
    admission: &TeamAdmission<'_>,
    request: &TeamRequest<'_>,
) -> Result<(), TeamBudgetError> {
    let (Some(team), Some(token)) = (request.team, request.token) else {
        return Ok(());
    };
    let Some(user_id) = token.view.token.user_id.as_deref() else {
        return Ok(());
    };
    let team_id = team.fields.base.team_id.as_str();
    let membership =
        resolve_membership(admission.memberships, &request.membership, user_id, team_id).await?;
    let budget_row = membership.as_ref().and_then(member_budget_row);
    let Some(cap) = member_cap(admission, team, budget_row).await? else {
        return Ok(());
    };
    let fallback = membership
        .as_ref()
        .and_then(|member| member.spend.0)
        .unwrap_or(Decimal::ZERO);
    let counter_key = format!("spend:team_member:{user_id}:{team_id}");
    let spend = read_counter(admission.reader, &counter_key, fallback, cap.finite_value()).await?;
    let alert = TeamMemberAlert {
        team_id,
        team_alias: team.fields.base.team_alias.as_deref(),
        team_metadata: team.fields.base.metadata.as_ref(),
        organization_id: team.fields.base.organization_id.as_deref(),
        user_id,
        user_email: request.user_email,
        spend,
        max_budget: cap,
    };
    enforce(admission.member_alerts, &alert)
}

// B751's member threshold alert runs before the denial, and the member is denied at equality
fn enforce(
    alerts: Option<&dyn BudgetAlertSink>,
    alert: &TeamMemberAlert<'_>,
) -> Result<(), TeamBudgetError> {
    // a maximum past Decimal's range is the infinity LiteLLM reads spend for and then skips
    let Some(maximum) = alert.max_budget.finite_value() else {
        return Ok(());
    };
    team_member_alert_check(alert, alerts);
    if alert.spend < maximum {
        return Ok(());
    }
    let (user_id, team_id, spend) = (alert.user_id, alert.team_id, alert.spend);
    Err(TeamBudgetError::Denied(Denial {
        current_cost: spend,
        max_budget: maximum,
        entity_type: "team_member".to_owned(),
        entity_id: Some(SecretString::from(format!("{user_id}:{team_id}"))),
        message: format!(
            "Budget has been exceeded! User={user_id} in Team={team_id} Current cost: {spend}, Max budget: {maximum}"
        ),
        status_code: BUDGET_EXCEEDED_STATUS,
    }))
}

// a nonnull member max_budget wins, zero included; otherwise only a positive team default caps, plus an active member increase
async fn member_cap(
    admission: &TeamAdmission<'_>,
    team: &LiteLLMTeamTable,
    budget_row: Option<&LiteLLMBudgetTable>,
) -> Result<Option<Money>, TeamBudgetError> {
    if let Some(row) = budget_row.filter(|row| row.max_budget.is_some()) {
        return Ok(match row.effective_max_budget(admission.now) {
            Ok(maximum) => maximum.map(Money::Finite),
            // a negative overflow is a finite cap in LiteLLM and denies every spend, while a positive one is the nonfinite maximum
            Err(BudgetError::Overflow)
                if row
                    .max_budget
                    .is_some_and(|maximum| maximum.is_sign_negative()) =>
            {
                Some(Money::Finite(Decimal::MIN))
            }
            Err(BudgetError::Overflow) => Some(Money::PositiveInfinity),
        });
    }
    let default_id = team
        .fields
        .base
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(TEAM_MEMBER_BUDGET_ID_FIELD));
    let Some(JsonData::String(budget_id)) = default_id else {
        return Ok(None);
    };
    let default_budget = admission.defaults.team_member_default(budget_id).await?;
    let Some(default_max) = default_budget
        .and_then(|budget| budget.max_budget)
        .filter(|maximum| *maximum > Decimal::ZERO)
    else {
        return Ok(None);
    };
    let increase = budget_row.map_or(Decimal::ZERO, |row| {
        row.active_temp_budget_increase(admission.now)
    });
    Ok(Some(
        Money::Finite(default_max).plus(Money::Finite(increase)),
    ))
}
