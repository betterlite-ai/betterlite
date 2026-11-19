use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::savings_evidence::metadata::Metadata;
use betterlite_models::savings_evidence::numeric::{numeric_savings, recorded_autorouter_savings};
use betterlite_models::tiered_pricing::Scalar;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::autorouter_turn_sql::transaction::{AutoRouterTurnTransaction, TurnFlag, TurnTime};

use super::cache::turn_cache_facts;
use super::session::bounded_session_id;
use super::turn_time::turn_time_utc;
use super::{AutorouterFactsError, INTERNAL_CALL_ORIGIN_KEY, counts_as_present};

// The SpendLogsPayload fields turn_transaction reads, plus the routing decision it reads from the metadata mapping.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TurnPayload {
    pub status: Option<String>,
    pub model: Option<String>,
    pub model_group: Option<String>,
    pub api_key: Option<SecretString>,
    pub user: Option<String>,
    pub session_id: Option<String>,
    #[serde(rename = "startTime")]
    pub start_time: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
}

// turn_transaction: one rollup transaction for a successful auto-routed turn, and None for everything the rollup must not count.
pub fn turn_transaction(
    payload: &TurnPayload,
    metadata: &JsonObject,
    saved_spend: Decimal,
) -> Result<Option<AutoRouterTurnTransaction>, AutorouterFactsError> {
    if payload.status.as_deref() != Some("success") {
        return Ok(None);
    }
    if metadata
        .get(INTERNAL_CALL_ORIGIN_KEY)
        .is_some_and(counts_as_present)
    {
        return Ok(None);
    }
    let Some(Json::Object(routing_decision)) = metadata.get("routing_decision") else {
        return Ok(None);
    };
    if routing_decision.is_empty() {
        return Ok(None);
    }
    let router_name = match routing_decision.get("router_model_name") {
        Some(value) if counts_as_present(value) => match value {
            Json::Text(name) => Some(name.as_str()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        },
        _ => payload.model_group.as_deref(),
    };
    let Some(router_name) = router_name.filter(|name| !name.is_empty()) else {
        return Ok(None);
    };
    let api_key = match &payload.api_key {
        Some(key) => key.expose_secret().to_owned(),
        None => String::new(),
    };
    let user_id = payload.user.clone().unwrap_or_default();
    let Some(session_id) = payload.session_id.as_deref().filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    let Some(model) = payload.model.as_deref().filter(|model| !model.is_empty()) else {
        return Ok(None);
    };
    if api_key.is_empty() && user_id.is_empty() {
        return Ok(None);
    }
    let Some(turn_at) = turn_time_utc(payload.start_time.as_deref().unwrap_or_default()) else {
        return Ok(None);
    };
    let usage_object = match metadata.get("usage_object") {
        Some(Json::Object(usage)) => Some(usage),
        _ => None,
    };
    let cache = turn_cache_facts(usage_object)?;
    let classifier_cost =
        numeric_from_json(routing_decision.get("classifier_cost")).unwrap_or_default();
    let actual_spend = payload
        .spend
        .unwrap_or(Decimal::ZERO)
        .checked_add(classifier_cost)
        .ok_or(AutorouterFactsError::SpendOverflow)?;
    let encoded_metadata = serde_json::to_vec(metadata).map_err(AutorouterFactsError::Encode)?;
    let decoded_metadata: Metadata =
        serde_json::from_slice(&encoded_metadata).map_err(AutorouterFactsError::Decode)?;
    let estimated_savings = recorded_autorouter_savings(&decoded_metadata);
    let total_tokens = payload
        .prompt_tokens
        .unwrap_or(0)
        .checked_add(payload.completion_tokens.unwrap_or(0))
        .ok_or(AutorouterFactsError::TokenTotalOverflow)?;
    let router_type = match routing_decision.get("router_type") {
        Some(Json::Text(name)) if !name.is_empty() => name.clone(),
        _ => "unknown".to_owned(),
    };
    let tier = match routing_decision.get("tier") {
        Some(Json::Text(name)) if !name.is_empty() => Some(name.clone()),
        _ => None,
    };
    let baseline_model = match routing_decision.get("savings_baseline_model") {
        Some(Json::Text(name)) if !name.is_empty() => Some(name.clone()),
        _ => None,
    };
    let (savings_estimated_turns, estimated_actual, estimated_saved) = match estimated_savings {
        Some(estimated) => (1, actual_spend, estimated),
        None => (0, Decimal::ZERO, Decimal::ZERO),
    };
    Ok(Some(AutoRouterTurnTransaction {
        api_key,
        session_id: bounded_session_id(session_id),
        router_name: router_name.to_owned(),
        router_type,
        model: model.to_owned(),
        turn_at: TurnTime::Naive(turn_at),
        total_tokens,
        spend: actual_spend,
        saved_spend,
        classifier_cost,
        covered: TurnFlag(cache.covered),
        cache_hit: TurnFlag(cache.read_tokens > 0),
        cache_ttl_seconds: cache.write_ttl_seconds,
        cache_touched: TurnFlag(cache.touched),
        tier,
        baseline_model,
        savings_estimated_turns,
        savings_estimated_actual_spend: estimated_actual,
        savings_estimated_saved_spend: estimated_saved,
        user_id,
    }))
}

// classifier_cost_from_decision reads only a number, since B1693's numeric savings excludes a bool and any other shape.
fn numeric_from_json(value: Option<&Json>) -> Option<Decimal> {
    let Some(Json::Number(number)) = value else {
        return None;
    };
    numeric_savings(Some(&Scalar::Number(number.clone())))
}

#[path = "turn/tests.rs"]
mod tests;
