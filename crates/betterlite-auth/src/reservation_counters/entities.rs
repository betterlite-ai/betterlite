use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::budget_primitives::window_start::DurationParser;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::CounterInput;
use super::windows::{WindowEntity, entry_window, payload_windows, window_counters};

pub(crate) fn key_counters(
    token: &UserAPIKeyAuth,
    durations: &dyn DurationParser,
    now: DateTime<Utc>,
) -> Vec<BudgetCounter> {
    let Some(secret) = token.view.token.token.as_ref() else {
        return Vec::new();
    };
    let key = &token.view.token;
    let token_text = secret.expose_secret();
    let mut counters = Vec::new();
    if let Some(max_budget) = key.max_budget.filter(|value| *value > Decimal::ZERO) {
        counters.push(BudgetCounter {
            counter_key: format!("spend:key:{token_text}"),
            max_budget,
            fallback_spend: key.spend,
            entity_type: CounterEntityType::Key,
            entity_id: token_text.to_owned(),
            source_cache_key: Some(token_text.to_owned()),
            spend_log_entity_id: None,
            window_duration: None,
            window_start: None,
        });
    }
    let windows = payload_windows(key.budget_limits.as_deref());
    let entity = WindowEntity {
        prefix: format!("spend:key:{token_text}"),
        entity_type: CounterEntityType::Key,
        entity_id: token_text.to_owned(),
    };
    counters.extend(window_counters(&entity, &windows, durations, now));
    counters
}

pub(crate) fn team_counters(
    team: Option<&LiteLLMTeamTable>,
    durations: &dyn DurationParser,
    now: DateTime<Utc>,
) -> Vec<BudgetCounter> {
    let Some(team) = team else {
        return Vec::new();
    };
    let base = &team.fields.base;
    let team_id = base.team_id.as_str();
    let mut counters = Vec::new();
    if let Some(max_budget) = base.max_budget.filter(|value| *value > Decimal::ZERO) {
        counters.push(BudgetCounter {
            counter_key: format!("spend:team:{team_id}"),
            max_budget,
            fallback_spend: team.fields.spend.unwrap_or_default(),
            entity_type: CounterEntityType::Team,
            entity_id: team_id.to_owned(),
            source_cache_key: Some(format!("team_id:{team_id}")),
            spend_log_entity_id: None,
            window_duration: None,
            window_start: None,
        });
    }
    let windows: Vec<JsonData> = base
        .budget_limits
        .iter()
        .flatten()
        .map(entry_window)
        .collect();
    let entity = WindowEntity {
        prefix: format!("spend:team:{team_id}"),
        entity_type: CounterEntityType::Team,
        entity_id: team_id.to_owned(),
    };
    counters.extend(window_counters(&entity, &windows, durations, now));
    counters
}

pub(crate) fn user_counter(input: &CounterInput<'_>) -> Option<BudgetCounter> {
    let user = input.user?;
    if input.team.is_some() && !input.apply_user_budget_to_team_keys {
        return None;
    }
    let max_budget = user.max_budget.filter(|value| *value > Decimal::ZERO)?;
    let user_id = user.user_id.as_str();
    Some(BudgetCounter {
        counter_key: format!("spend:user:{user_id}"),
        max_budget,
        fallback_spend: user.spend,
        entity_type: CounterEntityType::User,
        entity_id: user_id.to_owned(),
        source_cache_key: Some(user_id.to_owned()),
        spend_log_entity_id: None,
        window_duration: None,
        window_start: None,
    })
}
