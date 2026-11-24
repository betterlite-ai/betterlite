use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

use crate::budget_counter::CounterEntityType;
use crate::budget_primitives::window::{BudgetWindow, coerce_window};
use crate::budget_primitives::window_start::{DurationParser, budget_window_start};
use crate::current_spend::{SpendQuery, WindowDescriptors};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::KeyBudgetError;
use super::key_budget::{BUDGET_EXCEEDED_STATUS, Denial, token_text};
use super::money::Money;
use super::spend::SpendReader;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WindowError {
    #[error("budget_limits entry {index} has no budget_duration string")]
    Duration { index: usize },
    #[error("budget_limits entry {index} has no numeric max_budget")]
    MaxBudget { index: usize },
}

// _virtual_key_multi_budget_check: one counter per budget_duration, and the first window at or over its finite maximum denies
pub async fn virtual_key_multi_budget_check(
    auth: &UserAPIKeyAuth,
    reader: &dyn SpendReader,
    durations: &(dyn DurationParser + Sync),
    now: DateTime<Utc>,
) -> Result<(), KeyBudgetError> {
    let Some(limits) = auth.view.token.budget_limits.as_deref() else {
        return Ok(());
    };
    let token = token_text(auth);
    for (index, limit) in limits.iter().enumerate() {
        let window = coerce_window(&JsonData::Object(limit.0.clone()));
        let duration = window_duration(&window, index)?;
        let max_budget = window_max_budget(&window, index)?;
        let counter_key = format!("spend:key:{token}:window:{duration}");
        let start = budget_window_start(&window, now, durations).map(Timestamp::Aware);
        let query = SpendQuery {
            counter_key: &counter_key,
            fallback_spend: Decimal::ZERO,
            max_budget: max_budget.finite_value(),
            window: WindowDescriptors {
                entity_type: Some(&CounterEntityType::Key),
                entity_id: Some(&token),
                duration: Some(&duration),
                start,
            },
            fallback_authoritative: false,
        };
        let spend = reader.read_spend(&query).await?;
        if let Some(maximum) = max_budget.finite_value()
            && spend >= maximum
        {
            return Err(KeyBudgetError::Denied(window_denial(
                auth, &duration, spend, maximum,
            )));
        }
    }
    Ok(())
}

fn window_denial(
    auth: &UserAPIKeyAuth,
    duration: &str,
    spend: Decimal,
    maximum: Decimal,
) -> Denial {
    Denial {
        current_cost: spend,
        max_budget: maximum,
        entity_type: "key".to_owned(),
        entity_id: auth.view.token.token.clone(),
        message: format!(
            "ExceededBudget: Key over {duration} budget. Spend=${}, Limit=${}",
            format_scale(spend, 4),
            format_scale(maximum, 2)
        ),
        status_code: BUDGET_EXCEEDED_STATUS,
    }
}

fn window_duration(window: &BudgetWindow, index: usize) -> Result<String, WindowError> {
    match window.as_ref().get("budget_duration") {
        Some(JsonData::String(spec)) => Ok(spec.clone()),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => Err(WindowError::Duration { index }),
    }
}

fn window_max_budget(window: &BudgetWindow, index: usize) -> Result<Money, WindowError> {
    window
        .as_ref()
        .get("max_budget")
        .and_then(Money::from_json)
        .ok_or(WindowError::MaxBudget { index })
}

// the diagnostic's fixed four-decimal spend and two-decimal limit
fn format_scale(value: Decimal, scale: u32) -> String {
    let mut rounded = value.round_dp(scale);
    rounded.rescale(scale);
    rounded.to_string()
}
