use std::collections::BTreeMap;
use std::sync::PoisonError;

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
    let reads = effects
        .reads
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let writes = effects
        .writes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let increments = effects
        .increments
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let pipelines = effects
        .pipelines
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let mut read_values: Vec<Value> = Vec::new();
    for record in &reads {
        read_values.push(read_json(record));
    }
    let mut write_values: Vec<Value> = Vec::new();
    for record in &writes {
        write_values.push(write_json(record));
    }
    let mut increment_values: Vec<Value> = Vec::new();
    for operation in &increments {
        increment_values.push(increment_json(operation));
    }
    let mut pipeline_values: Vec<Value> = Vec::new();
    for operations in &pipelines {
        pipeline_values.push(Value::Array(
            operations.iter().map(increment_json).collect(),
        ));
    }
    json!({
        "reads": read_values,
        "writes": write_values,
        "increments": increment_values,
        "pipelines": pipeline_values,
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
        "headers": headers,
        "rate_limit_type": rejection.rate_limit_type,
        "category": rejection.category,
    }))
}
