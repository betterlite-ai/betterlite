use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::auth_key_budgets::key_budget::{BUDGET_EXCEEDED_STATUS, Denial};

use super::alerts::{TeamAlertKind, token_alert};
use super::error::TeamBudgetError;
use super::sources::read_counter;
use super::{TeamAdmission, TeamRequest};

// _team_max_budget_check: equality passes, and a token queues team_budget before the denial
pub async fn team_max_budget_check(
    admission: &TeamAdmission<'_>,
    request: &TeamRequest<'_>,
) -> Result<(), TeamBudgetError> {
    let Some(team) = request.team else {
        return Ok(());
    };
    let Some(max_budget) = team.fields.base.max_budget else {
        return Ok(());
    };
    let team_id = team.fields.base.team_id.as_str();
    let counter_key = format!("spend:team:{team_id}");
    let fallback = team.fields.spend.unwrap_or(Decimal::ZERO);
    let spend = read_counter(admission.reader, &counter_key, fallback, Some(max_budget)).await?;
    if spend <= max_budget {
        return Ok(());
    }
    if let (Some(token), Some(sink)) = (request.token, admission.team_alerts) {
        sink.schedule(token_alert(
            TeamAlertKind::TeamBudget,
            token,
            team,
            spend,
            None,
        ));
    }
    Err(TeamBudgetError::Denied(Denial {
        current_cost: spend,
        max_budget,
        entity_type: "team".to_owned(),
        entity_id: Some(SecretString::from(team_id.to_owned())),
        message: format!(
            "Budget has been exceeded! Team={team_id} Current cost: {spend}, Max budget: {max_budget}"
        ),
        status_code: BUDGET_EXCEEDED_STATUS,
    }))
}
