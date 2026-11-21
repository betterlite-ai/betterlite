use std::collections::BTreeMap;

use betterlite_rate_limit::limiter_records::{
    CacheCounterValue, RateLimitResponse, ReservationWindowIdentity,
};
use betterlite_rate_limit::token_reservations::TokenReservationError;
use betterlite_redis::cache_primitives::pipeline_increment::IncrementOperation;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use super::fakes::{Effects, ReadRecord, RecordedValue, WriteRecord};

fn counter_value_json(value: &CacheCounterValue) -> Value {
    match value {
        CacheCounterValue::Int(number) => json!(number),
        CacheCounterValue::Float(number) => json!(number),
        CacheCounterValue::Text(text) => json!(text),
        CacheCounterValue::Bytes(bytes) => json!(bytes),
    }
}

fn read_json(record: &ReadRecord) -> Value {
    json!({
        "key": record.key.clone(),
        "local_only": record.local_only,
    })
}

fn write_json(record: &WriteRecord) -> Value {
    let value = match &record.value {
        RecordedValue::Counter(counter) => counter_value_json(counter),
        RecordedValue::Local(number) => json!(number),
    };
    json!({
        "key": record.key.clone(),
        "value": value,
        "ttl": record.ttl,
        "local_only": record.local_only,
    })
}

fn increment_json(operation: &IncrementOperation) -> Value {
    json!({
        "key": operation.key.clone(),
        "increment_value": operation.increment_value,
        "ttl": operation.ttl,
    })
}

pub(super) fn effects_json(effects: &Effects) -> Value {
    let mut reads: Vec<Value> = Vec::new();
    for record in effects.reads() {
        reads.push(read_json(&record));
    }
    let mut writes: Vec<Value> = Vec::new();
    for record in effects.writes() {
        writes.push(write_json(&record));
    }
    let mut increments: Vec<Value> = Vec::new();
    for operation in effects.increments() {
        increments.push(increment_json(&operation));
    }
    let mut pipelines: Vec<Value> = Vec::new();
    for operations in effects.pipelines() {
        pipelines.push(Value::Array(
            operations.iter().map(increment_json).collect(),
        ));
    }
    json!({
        "reads": reads,
        "writes": writes,
        "increments": increments,
        "pipelines": pipelines,
    })
}

pub(super) fn response_json(response: &RateLimitResponse) -> Value {
    let windows: Vec<&ReservationWindowIdentity> =
        response.reservation_windows.iter().flatten().collect();
    json!({
        "overall_code": response.overall_code.clone(),
        "statuses": response.statuses.clone(),
        "reservation_windows": windows,
    })
}

pub(super) fn raised_json(error: &TokenReservationError) -> Result<Value, Box<HelperError>> {
    let TokenReservationError::RateLimited(rejection) = error else {
        return Err(error).or_value_error();
    };
    let headers = match &rejection.headers {
        None => Value::Null,
        Some(present) => {
            let mut collected: BTreeMap<String, String> = BTreeMap::new();
            for (name, value) in present {
                let text = value.to_str().or_value_error()?;
                collected.insert(name.as_str().to_owned(), text.to_owned());
            }
            json!(collected)
        }
    };
    Ok(json!({
        "status": 429_u16,
        "detail": rejection.detail.clone(),
        "headers": headers,
        "rate_limit_type": rejection.rate_limit_type,
        "category": rejection.category,
    }))
}
