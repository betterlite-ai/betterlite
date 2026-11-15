use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::shadow_tables::rows::ShadowAttemptRow;
use betterlite_server::shadow_eval_job_loading::attempt::record_attempt;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

use super::inputs::{AttemptCall, RowInput};
use super::support::{AttemptTables, FakeCounter, build_runtime, failure, outcome};

pub(crate) fn record(call: &AttemptCall) -> HelperResult {
    let runtime = build_runtime()?;
    let events = Arc::new(Mutex::new(Vec::new()));
    let tables = AttemptTables {
        events: Arc::clone(&events),
        inserts: Mutex::new(Vec::new()),
    };
    let counter = FakeCounter {
        events: Arc::clone(&events),
    };
    let attempt = attempt_row(&call.job.id, &call.row)?;
    runtime.block_on(record_attempt(Some(&tables), &counter, attempt));
    let stored = tables
        .inserts
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .cloned();
    let Some(stored) = stored else {
        return Err(failure("RuntimeError", "no attempt row was inserted"));
    };
    Ok(json!({
        "events": events.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        "row": row_json(&stored),
        "funnel": Vec::<Value>::new(),
    }))
}

fn attempt_row(job_id: &str, row: &RowInput) -> Result<ShadowAttemptRow, Box<HelperError>> {
    Ok(ShadowAttemptRow {
        job_id: job_id.to_owned(),
        request_id: row.request_id.clone(),
        router_name: row.router_name.clone(),
        outcome: outcome(&row.outcome)?,
        tier: row.tier.clone(),
        real_model: Some(row.real_model.clone()),
        shadow_model: row.shadow_model.clone(),
        confidence: row.confidence,
        judge_cost: row.judge_cost,
        shadow_cost: row.shadow_cost,
        shadow_classifier_cost: row.shadow_classifier_cost,
        real_cost: Some(row.real_cost),
        real_classifier_cost: row.real_classifier_cost,
        real_cache_hit: row.real_cache_hit,
        error: row.error.clone(),
    })
}

fn row_json(stored: &ShadowAttemptRow) -> Value {
    json!({
        "request_id": stored.request_id,
        "outcome": stored.outcome.as_str(),
        "tier": stored.tier,
        "real_model": stored.real_model,
        "shadow_model": stored.shadow_model,
        "confidence": stored.confidence.map(|value| value.normalize().to_string()),
        "judge_cost": stored.judge_cost.normalize().to_string(),
        "shadow_cost": stored.shadow_cost.normalize().to_string(),
        "shadow_classifier_cost": stored.shadow_classifier_cost.normalize().to_string(),
        "real_cost": stored.real_cost.map(|value| value.normalize().to_string()),
        "real_classifier_cost": stored.real_classifier_cost.normalize().to_string(),
        "real_cache_hit": stored.real_cache_hit,
        "error_chars": stored.error.as_ref().map_or(0, |error| error.chars().count()),
    })
}
