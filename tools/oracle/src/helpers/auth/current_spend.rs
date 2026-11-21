mod fakes;
mod input;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_auth::budget_counter::CounterEntityType;
use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::current_spend::{
    CurrentSpend, CurrentSpendError, SpendQuery, WindowDescriptors,
};
use betterlite_auth::model_budget_limiter::clock::SystemClock as UptimeClock;
use betterlite_auth::spend_reseed::locks::LockRegistry;
use betterlite_auth::spend_reseed::reseed::SpendCounterReseed;
use betterlite_auth::spend_reseed::values::ReseedCacheValue;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio_util::task::TaskTracker;

use fakes::{
    Memory, ScriptedBatch, ScriptedCache, ScriptedFailure, ScriptedRows, amount_json, memory_json,
};
use input::{Input, Operation};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Builder::new_current_thread()
            .enable_time()
            .build()
            .or_value_error()?
            .block_on(execute(input))
    }
}

// duration_in_seconds for the window suffixes the cases use, so a window counter skips the entity read as in LiteLLM
struct CaseDurations;

impl DurationParser for CaseDurations {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        match spec {
            "1d" => Some(86_400),
            _ => None,
        }
    }
}

fn case_reseed(rows: Option<ScriptedRows>) -> SpendCounterReseed<ScriptedRows> {
    let gate = Arc::new(LoopBoundSemaphore {
        value: 25,
        slot: Mutex::new(None),
    });
    let stall_tracker = Arc::new(DbLookupStallTracker {
        clock: SystemClock,
        last_hit: Mutex::new(None),
    });
    SpendCounterReseed {
        rows: rows.map(Arc::new),
        locks: LockRegistry::from(1_000_i64),
        clock: Box::new(UptimeClock {
            started: Instant::now(),
        }),
        durations: Box::new(CaseDurations),
        tasks: TaskTracker::new(),
        gate,
        stall_tracker,
        deadline_seconds: 5.0_f64,
    }
}

// get_current_spend raises HTTPException(503) for an unverifiable strict check and lets every other error through
fn spend_error(error: &CurrentSpendError<ScriptedFailure>) -> Box<HelperError> {
    let (class, status) = match error {
        CurrentSpendError::Unverifiable { .. } => ("HTTPException", Some(503)),
        CurrentSpendError::Cache(_)
        | CurrentSpendError::Value(_)
        | CurrentSpendError::Batch(_)
        | CurrentSpendError::Reseed(_) => ("ValueError", None),
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

fn seeded_memory(input: &Input, marker_key: &str) -> Memory {
    let mut seeded = BTreeMap::new();
    if let Some(text) = &input.memory {
        seeded.insert(
            input.counter_key.clone(),
            (ReseedCacheValue::Text(text.clone()), None),
        );
    }
    if let Some(marker) = input.marker {
        seeded.insert(
            marker_key.to_owned(),
            (ReseedCacheValue::Number(marker), None),
        );
    }
    Arc::new(Mutex::new(seeded))
}

fn window_start(input: &Input) -> Result<Option<Timestamp>, Box<HelperError>> {
    input
        .window_start
        .as_deref()
        .map(DateTime::parse_from_rfc3339)
        .transpose()
        .map(|parsed| parsed.map(Timestamp::Aware))
        .or_value_error()
}

fn marker_ttl(memory: &Memory, marker_key: &str) -> Option<u32> {
    memory
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(marker_key)
        .and_then(|(_value, ttl)| *ttl)
}

async fn execute(input: &Input) -> HelperResult {
    let marker_key = format!("spend_db_floor:{}", input.counter_key);
    let memory = seeded_memory(input, &marker_key);
    let start = window_start(input)?;
    let reseed = case_reseed(input.rows.clone().map(|rows| ScriptedRows {
        rows,
        memory: Arc::clone(&memory),
        marker_key: marker_key.clone(),
        window_start: start,
    }));
    let cache = ScriptedCache {
        input,
        memory: Arc::clone(&memory),
        set_max_calls: Mutex::new(Vec::new()),
    };
    let batch = ScriptedBatch::open(input);
    let service = CurrentSpend {
        cache: &cache,
        batch: &batch,
        reseed: &reseed,
        fail_closed_budget_enforcement: input.strict.0,
    };
    let result = answer(&service, input, start).await?;
    let set_max_calls = cache
        .set_max_calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(json!({
        "result": result,
        "memory_after": memory_json(&memory, &input.counter_key),
        "marker_after": memory_json(&memory, &marker_key),
        "marker_ttl": marker_ttl(&memory, &marker_key),
        "set_max_calls": set_max_calls,
        "batch_after": batch.after(),
    }))
}

type CaseService<'deps> = CurrentSpend<'deps, ScriptedCache<'deps>, ScriptedBatch, ScriptedRows>;

async fn answer(
    service: &CaseService<'_>,
    input: &Input,
    start: Option<Timestamp>,
) -> HelperResult {
    let entity_type = input
        .window_entity_type
        .as_deref()
        .and_then(CounterEntityType::from_litellm_name);
    let query = SpendQuery {
        counter_key: &input.counter_key,
        fallback_spend: input.fallback_spend,
        max_budget: input.max_budget,
        window: WindowDescriptors {
            entity_type: entity_type.as_ref(),
            entity_id: input.window_entity_id.as_deref(),
            duration: input.window_duration.as_deref(),
            start,
        },
        fallback_authoritative: input.fallback_authoritative.0,
    };
    match input.op {
        Operation::GetCurrentSpend => service
            .get_current_spend(&query)
            .await
            .map(|spend| json!({"spend": amount_json(spend)})),
        Operation::ReadSpendCounterCacheValue => service
            .read_spend_counter_cache_value(&input.counter_key)
            .await
            .map(|reading| {
                let value = reading.value.map_or(Value::Null, amount_json);
                json!({"value": value, "authoritative": reading.authoritative})
            }),
    }
    .map_err(|error| spend_error(&error))
}
