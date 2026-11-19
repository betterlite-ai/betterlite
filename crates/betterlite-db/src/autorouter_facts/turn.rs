use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::savings_evidence::metadata::Metadata;
use betterlite_models::savings_evidence::numeric::{numeric_savings, recorded_autorouter_savings};
use betterlite_models::tiered_pricing::Scalar;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::autorouter_turn_sql::transaction::{AutoRouterTurnTransaction, TurnFlag, TurnTime};

use super::cache::turn_cache_facts;
use super::session::bounded_session_id;
use super::turn_time::turn_time_utc;
use super::{AutorouterFactsError, INTERNAL_CALL_ORIGIN_KEY, counts_as_present};

// The SpendLogsPayload fields turn_transaction reads, plus the routing decision it reads from the metadata mapping.
#[derive(Debug, Default, Deserialize)]
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

// TurnIdentity: the payload fields a countable turn must carry, with the session id already bounded for the column.
struct TurnIdentity {
    api_key: String,
    user_id: String,
    session_id: String,
    model: String,
}

// SavingsSnapshot: the savings a turn records, one estimated turn charged its own spend or zeros when nothing was estimated.
#[derive(Debug)]
struct SavingsSnapshot {
    turns: i64,
    actual_spend: Decimal,
    saved_spend: Decimal,
}

// turn_transaction: one rollup transaction for a successful auto-routed turn, and None for everything the rollup must not count.
pub fn turn_transaction(
    payload: &TurnPayload,
    metadata: &JsonObject,
    saved_spend: Decimal,
) -> Result<Option<AutoRouterTurnTransaction>, AutorouterFactsError> {
    let Some((decision, router_name, identity, turn_at)) = counted_turn(payload, metadata) else {
        return Ok(None);
    };
    let cache = turn_cache_facts(usage_object(metadata))?;
    let classifier_cost = numeric_from_json(decision.get("classifier_cost")).unwrap_or_default();
    let (spend, savings) = turn_savings(payload, metadata, classifier_cost)?;
    Ok(Some(AutoRouterTurnTransaction {
        api_key: identity.api_key,
        session_id: identity.session_id,
        router_name: router_name.to_owned(),
        router_type: non_empty_text(decision.get("router_type"))
            .map_or_else(|| "unknown".to_owned(), str::to_owned),
        model: identity.model,
        turn_at: TurnTime::Naive(turn_at),
        total_tokens: turn_tokens(payload)?,
        spend,
        saved_spend,
        classifier_cost,
        covered: TurnFlag(cache.covered),
        cache_hit: TurnFlag(cache.read_tokens > 0),
        cache_ttl_seconds: cache.write_ttl_seconds,
        cache_touched: TurnFlag(cache.touched),
        tier: non_empty_text(decision.get("tier")).map(str::to_owned),
        baseline_model: non_empty_text(decision.get("savings_baseline_model")).map(str::to_owned),
        savings_estimated_turns: savings.turns,
        savings_estimated_actual_spend: savings.actual_spend,
        savings_estimated_saved_spend: savings.saved_spend,
        user_id: identity.user_id,
    }))
}

// counted_turn: the routing decision, its rollup name, the payload identity and the turn time of a turn the rollup counts.
fn counted_turn<'turn>(
    payload: &'turn TurnPayload,
    metadata: &'turn JsonObject,
) -> Option<(&'turn JsonObject, &'turn str, TurnIdentity, NaiveDateTime)> {
    if payload.status.as_deref() != Some("success") {
        return None;
    }
    let (routing_decision, router_name) = counted_decision(payload, metadata)?;
    let identity = turn_identity(payload)?;
    let turn_at = turn_time_utc(payload.start_time.as_deref().unwrap_or_default())?;
    Some((routing_decision, router_name, identity, turn_at))
}

// counted_decision: the routing decision a countable turn carries, with the name the rollup files it under.
fn counted_decision<'turn>(
    payload: &'turn TurnPayload,
    metadata: &'turn JsonObject,
) -> Option<(&'turn JsonObject, &'turn str)> {
    if metadata
        .get(INTERNAL_CALL_ORIGIN_KEY)
        .is_some_and(counts_as_present)
    {
        return None;
    }
    let Some(Json::Object(routing_decision)) = metadata.get("routing_decision") else {
        return None;
    };
    if routing_decision.is_empty() {
        return None;
    }
    let router_name = match routing_decision.get("router_model_name") {
        Some(value) if counts_as_present(value) => non_empty_text(Some(value))?,
        _ => payload
            .model_group
            .as_deref()
            .filter(|name| !name.is_empty())?,
    };
    Some((routing_decision, router_name))
}

// turn_identity: the key, user, session and model a row needs, and None when the session, the model or both the key and the user are missing.
fn turn_identity(payload: &TurnPayload) -> Option<TurnIdentity> {
    let api_key = payload
        .api_key
        .as_ref()
        .map_or_else(String::new, |key| key.expose_secret().to_owned());
    let user_id = payload.user.clone().unwrap_or_default();
    let session_id = payload.session_id.as_deref().filter(|id| !id.is_empty())?;
    let model = payload.model.as_deref().filter(|model| !model.is_empty())?;
    if api_key.is_empty() && user_id.is_empty() {
        return None;
    }
    Some(TurnIdentity {
        api_key,
        user_id,
        session_id: bounded_session_id(session_id),
        model: model.to_owned(),
    })
}

// turn_savings: the payload's spend with the classifier cost added, and the savings the metadata records for it.
fn turn_savings(
    payload: &TurnPayload,
    metadata: &JsonObject,
    classifier_cost: Decimal,
) -> Result<(Decimal, SavingsSnapshot), AutorouterFactsError> {
    let spend = payload
        .spend
        .unwrap_or(Decimal::ZERO)
        .checked_add(classifier_cost)
        .ok_or(AutorouterFactsError::SpendOverflow)?;
    let encoded = serde_json::to_vec(metadata).map_err(AutorouterFactsError::Encode)?;
    let decoded: Metadata =
        serde_json::from_slice(&encoded).map_err(AutorouterFactsError::Decode)?;
    let snapshot = recorded_autorouter_savings(&decoded).map_or(
        SavingsSnapshot {
            turns: 0,
            actual_spend: Decimal::ZERO,
            saved_spend: Decimal::ZERO,
        },
        |estimated| SavingsSnapshot {
            turns: 1,
            actual_spend: spend,
            saved_spend: estimated,
        },
    );
    Ok((spend, snapshot))
}

// turn_tokens: the prompt and completion counts the payload carries, and an overflow error when their sum does not fit.
fn turn_tokens(payload: &TurnPayload) -> Result<i64, AutorouterFactsError> {
    payload
        .prompt_tokens
        .unwrap_or(0)
        .checked_add(payload.completion_tokens.unwrap_or(0))
        .ok_or(AutorouterFactsError::TokenTotalOverflow)
}

// usage_object: the usage telemetry a turn carries, and None when the metadata has none or a non-object value.
fn usage_object(metadata: &JsonObject) -> Option<&JsonObject> {
    match metadata.get("usage_object") {
        Some(Json::Object(usage)) => Some(usage),
        _ => None,
    }
}

// non_empty_text: the text of a JSON string that names something, and None for a missing, empty or non-text value.
fn non_empty_text(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
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
