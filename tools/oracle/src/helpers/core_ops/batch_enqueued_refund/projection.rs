use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_core::api::batch_enqueued_refund::BatchEnqueuedRefundResult;
use betterlite_core::api::batch_enqueued_refund::record::ReservationRecord;
use betterlite_core::api::batch_enqueued_reserve::BatchEnqueuedTokenScope;
use serde_json::{Map, Value, json};

use super::error::failure;
use super::local_store::RecordingStore;
use crate::helpers::HelperError;

fn cached_value(value: CacheValue) -> Value {
    match value {
        CacheValue::Text(text) => match serde_json::from_str::<Value>(&text) {
            Ok(parsed) => parsed,
            Err(_error) => Value::String(text),
        },
        other => serde_json::to_value(other).unwrap_or_else(|_error| Value::Null),
    }
}

pub(super) fn read_memory(
    store: &Arc<Mutex<RecordingStore>>,
    keys: &[String],
) -> Result<Map<String, Value>, Box<HelperError>> {
    let mut held = store.lock().unwrap_or_else(PoisonError::into_inner);
    let mut memory = Map::new();
    for key in keys {
        let stored = held
            .counters
            .cache
            .get(key)
            .map_err(|error| failure(error.to_string()))?;
        memory.insert(key.clone(), stored.map_or(Value::Null, cached_value));
    }
    Ok(memory)
}

fn scope_value(scope: &BatchEnqueuedTokenScope) -> Value {
    json!({
        "key": scope.key,
        "value": scope.value,
        "limit": scope.limit,
    })
}

fn record_value(record: &ReservationRecord) -> Value {
    json!({
        "tokens": record.tokens,
        "scopes": record.scopes.iter().map(scope_value).collect::<Vec<_>>(),
        "backend": record.backend,
        "owner": record.owner,
        "reserved_at_monotonic": record.reserved_at_monotonic,
    })
}

pub(super) fn outcome_value(result: &BatchEnqueuedRefundResult) -> Value {
    match result {
        BatchEnqueuedRefundResult::PostCall(outcome) => json!({
            "kind": "post_call",
            "batch_id": outcome.batch_id,
            "saved": outcome.saved,
            "refunded": outcome.refunded,
        }),
        BatchEnqueuedRefundResult::Saved => json!({"kind": "saved"}),
        BatchEnqueuedRefundResult::Popped(reservation) => json!({
            "kind": "popped",
            "reservation": reservation.as_ref().map(record_value),
        }),
        BatchEnqueuedRefundResult::Refunded => json!({"kind": "refunded"}),
    }
}
