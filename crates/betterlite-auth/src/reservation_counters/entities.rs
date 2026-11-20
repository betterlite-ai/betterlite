use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::budget_primitives::window_start::DurationParser;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::CounterInput;
use super::windows::{WindowEntity, entry_window, payload_windows, window_counters};

// the counter prefix and id a non-window counter names, kept apart from its budget so the counter fields are written once
pub(crate) struct CounterEntity {
    pub(crate) prefix: String,
    pub(crate) entity_type: CounterEntityType,
    pub(crate) entity_id: String,
    pub(crate) source_cache_key: String,
}

pub(crate) fn entity_counter(
    entity: &CounterEntity,
    max_budget: Decimal,
    fallback_spend: Decimal,
) -> BudgetCounter {
    BudgetCounter {
        counter_key: entity.prefix.clone(),
        max_budget,
        fallback_spend,
        entity_type: entity.entity_type.clone(),
        entity_id: entity.entity_id.clone(),
        source_cache_key: Some(entity.source_cache_key.clone()),
        spend_log_entity_id: None,
        window_duration: None,
        window_start: None,
    }
}

pub(crate) fn entity_windows(
    entity: &CounterEntity,
    windows: &[JsonData],
    durations: &dyn DurationParser,
    now: DateTime<Utc>,
) -> Vec<BudgetCounter> {
    window_counters(
        &WindowEntity {
            prefix: entity.prefix.clone(),
            entity_type: entity.entity_type.clone(),
            entity_id: entity.entity_id.clone(),
        },
        windows,
        durations,
        now,
    )
}

// LiteLLM collects the key counter and then its windows
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
    let entity = CounterEntity {
        prefix: format!("spend:key:{token_text}"),
        entity_type: CounterEntityType::Key,
        entity_id: token_text.to_owned(),
        source_cache_key: token_text.to_owned(),
    };
    let mut counters = Vec::new();
    if let Some(max_budget) = key.max_budget.filter(|value| *value > Decimal::ZERO) {
        counters.push(entity_counter(&entity, max_budget, key.spend));
    }
    let windows = payload_windows(key.budget_limits.as_deref());
    counters.extend(entity_windows(&entity, &windows, durations, now));
    counters
}

// LiteLLM collects the team counter and then its windows
pub(crate) fn team_counters(
    team: Option<&LiteLLMTeamTable>,
    durations: &dyn DurationParser,
    now: DateTime<Utc>,
) -> Vec<BudgetCounter> {
    let Some(team_row) = team else {
        return Vec::new();
    };
    let base = &team_row.fields.base;
    let team_id = base.team_id.as_str();
    let windows: Vec<JsonData> = base
        .budget_limits
        .iter()
        .flatten()
        .map(entry_window)
        .collect();
    let entity = CounterEntity {
        prefix: format!("spend:team:{team_id}"),
        entity_type: CounterEntityType::Team,
        entity_id: team_id.to_owned(),
        source_cache_key: format!("team_id:{team_id}"),
    };
    let mut counters = Vec::new();
    if let Some(max_budget) = base.max_budget.filter(|value| *value > Decimal::ZERO) {
        counters.push(entity_counter(
            &entity,
            max_budget,
            team_row.fields.spend.unwrap_or_default(),
        ));
    }
    counters.extend(entity_windows(&entity, &windows, durations, now));
    counters
}

pub(crate) fn user_counter(input: &CounterInput<'_>) -> Option<BudgetCounter> {
    let user = input.user?;
    if input.team.is_some() && !input.apply_user_budget_to_team_keys {
        return None;
    }
    let max_budget = user.max_budget.filter(|value| *value > Decimal::ZERO)?;
    let user_id = user.user_id.as_str();
    let entity = CounterEntity {
        prefix: format!("spend:user:{user_id}"),
        entity_type: CounterEntityType::User,
        entity_id: user_id.to_owned(),
        source_cache_key: user_id.to_owned(),
    };
    Some(entity_counter(&entity, max_budget, user.spend))
}
