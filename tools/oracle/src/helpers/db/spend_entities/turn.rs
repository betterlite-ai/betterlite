use betterlite_db::autorouter_facts::turn::{TurnPayload, turn_transaction};
use betterlite_db::autorouter_turn_sql::transaction::{AutoRouterTurnTransaction, TurnTime};
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Value;

use super::{Step, failure};

#[derive(Debug, Serialize)]
struct TurnAnswer {
    record: Option<TurnView>,
}

#[derive(Debug, Serialize)]
struct TurnView {
    api_key: String,
    session_id: String,
    router_name: String,
    router_type: String,
    model: String,
    turn_at: String,
    total_tokens: i64,
    spend: String,
    saved_spend: String,
    classifier_cost: String,
    covered: bool,
    cache_hit: bool,
    cache_ttl_seconds: Option<i64>,
    cache_touched: bool,
    tier: Option<String>,
    baseline_model: Option<String>,
    savings_estimated_turns: i64,
    savings_estimated_actual_spend: String,
    savings_estimated_saved_spend: String,
    user_id: String,
}

pub(super) fn turn_value(
    step: &Step,
) -> Result<Value, Box<betterlite_oracle_helper_api::error::HelperError>> {
    let payload = step
        .payload
        .clone()
        .ok_or_else(|| failure("a turn step needs a payload"))?;
    let payload: TurnPayload = serde_json::from_value(payload).map_err(failure)?;
    let metadata_value = step
        .metadata
        .clone()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let metadata: JsonObject = serde_json::from_value(metadata_value).map_err(failure)?;
    let saved_spend = match &step.saved_spend {
        None | Some(Value::Null) => Decimal::ZERO,
        Some(value) => serde_json::from_value::<Decimal>(value.clone()).map_err(failure)?,
    };
    let record = turn_transaction(&payload, &metadata, saved_spend)
        .map_err(|error| failure(error.to_string()))?;
    serde_json::to_value(TurnAnswer {
        record: record.map(turn_view),
    })
    .map_err(failure)
}

fn turn_view(record: AutoRouterTurnTransaction) -> TurnView {
    TurnView {
        api_key: record.api_key,
        session_id: record.session_id,
        router_name: record.router_name,
        router_type: record.router_type,
        model: record.model,
        turn_at: turn_at_text(record.turn_at),
        total_tokens: record.total_tokens,
        spend: format!("{:.10}", record.spend),
        saved_spend: format!("{:.10}", record.saved_spend),
        classifier_cost: format!("{:.10}", record.classifier_cost),
        covered: record.covered.0,
        cache_hit: record.cache_hit.0,
        cache_ttl_seconds: record.cache_ttl_seconds,
        cache_touched: record.cache_touched.0,
        tier: record.tier,
        baseline_model: record.baseline_model,
        savings_estimated_turns: record.savings_estimated_turns,
        savings_estimated_actual_spend: format!("{:.10}", record.savings_estimated_actual_spend),
        savings_estimated_saved_spend: format!("{:.10}", record.savings_estimated_saved_spend),
        user_id: record.user_id,
    }
}

// The naive UTC datetime as the source's ``datetime.isoformat()`` spells it.
fn turn_at_text(turn_at: TurnTime) -> String {
    let time = match turn_at {
        TurnTime::Naive(time) => time,
        TurnTime::Aware(time) => time.naive_utc(),
    };
    let pattern = if time.and_utc().timestamp_subsec_micros() == 0 {
        "%Y-%m-%dT%H:%M:%S"
    } else {
        "%Y-%m-%dT%H:%M:%S%.6f"
    };
    time.format(pattern).to_string()
}
