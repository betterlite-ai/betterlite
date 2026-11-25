use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::team_rows::member::BudgetLimitEntry;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::auth_key_budgets::key_budget::{BUDGET_EXCEEDED_STATUS, Denial};
use crate::budget_counter::CounterEntityType;
use crate::budget_primitives::window::coerce_window;
use crate::budget_primitives::window_start::budget_window_start;
use crate::current_spend::{SpendQuery, WindowDescriptors};

use super::TeamAdmission;
use super::error::TeamBudgetError;

// _team_multi_budget_check: one counter per budget_duration, in list order, and the first window at its maximum denies
pub async fn team_multi_budget_check(
    admission: &TeamAdmission<'_>,
    team: Option<&LiteLLMTeamTable>,
) -> Result<(), TeamBudgetError> {
    let Some(team) = team else {
        return Ok(());
    };
    let Some(limits) = team.fields.base.budget_limits.as_deref() else {
        return Ok(());
    };
    let team_id = team.fields.base.team_id.as_str();
    for limit in limits {
        let duration = limit.budget_duration.as_str();
        let counter_key = format!("spend:team:{team_id}:window:{duration}");
        let window = coerce_window(&JsonData::Object(window_fields(limit)));
        let query = SpendQuery {
            counter_key: &counter_key,
            fallback_spend: Decimal::ZERO,
            max_budget: Some(limit.max_budget),
            window: WindowDescriptors {
                entity_type: Some(&CounterEntityType::Team),
                entity_id: Some(team_id),
                duration: Some(duration),
                start: budget_window_start(&window, admission.now, admission.durations)
                    .map(Timestamp::Aware),
            },
            fallback_authoritative: false,
        };
        let spend = admission.reader.read_spend(&query).await?;
        if spend >= limit.max_budget {
            return Err(TeamBudgetError::Denied(window_denial(
                team_id, limit, spend,
            )));
        }
    }
    Ok(())
}

// the two fields get_budget_window_start reads off the window; an aware reset keeps its offset and a naive one is UTC
fn window_fields(limit: &BudgetLimitEntry) -> BTreeMap<String, JsonData> {
    let mut fields = BTreeMap::from([(
        "budget_duration".to_owned(),
        JsonData::String(limit.budget_duration.clone()),
    )]);
    let reset_at = match limit.reset_at {
        Some(Timestamp::Aware(moment)) => moment.to_rfc3339(),
        Some(Timestamp::Naive(moment)) => moment.and_utc().to_rfc3339(),
        None => return fields,
    };
    fields.insert("reset_at".to_owned(), JsonData::String(reset_at));
    fields
}

fn window_denial(team_id: &str, limit: &BudgetLimitEntry, spend: Decimal) -> Denial {
    Denial {
        current_cost: spend,
        max_budget: limit.max_budget,
        entity_type: "team".to_owned(),
        entity_id: Some(SecretString::from(team_id.to_owned())),
        message: format!(
            "ExceededBudget: Team={team_id} over {} budget. Spend=${}, Limit=${}",
            limit.budget_duration,
            fixed_point(spend, 4),
            fixed_point(limit.max_budget, 2)
        ),
        status_code: BUDGET_EXCEEDED_STATUS,
    }
}

// Python's :.4f and :.2f, rounding half to even at the given scale and padding with zeros
fn fixed_point(value: Decimal, scale: u32) -> String {
    let mut shown = value.round_dp(scale);
    shown.rescale(scale);
    shown.to_string()
}
