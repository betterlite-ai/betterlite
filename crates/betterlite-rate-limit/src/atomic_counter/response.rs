use std::collections::BTreeSet;

use crate::limiter_records::{
    AtomicCounterMeta, CacheCounterValue, RateLimitResponse, RateLimitStatus, ReservationBackend,
    ReservationWindowIdentity,
};

use super::rate_limit_kind::status_rate_limit_type;
use super::value::{CounterValueError, counter_integer};

mod tests;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum ResponseDecodeError {
    #[error("the Lua reply has no element at position {position}")]
    MissingElement { position: usize },
    #[error("the Lua reply names counter index {index} but there are {count} counters")]
    CounterIndex { index: i64, count: usize },
    #[error(transparent)]
    Value(#[from] CounterValueError),
}

pub fn build_atomic_response(
    raw: &[CacheCounterValue],
    per_counter_meta: &[AtomicCounterMeta],
) -> Result<RateLimitResponse, ResponseDecodeError> {
    let Some(first) = raw.first() else {
        return Ok(RateLimitResponse {
            overall_code: "OK".to_owned(),
            statuses: Vec::new(),
            reservation_windows: None,
        });
    };
    if counter_integer("reply[0]", first)? == 1 {
        return over_limit_response(raw, per_counter_meta);
    }
    success_response(raw, per_counter_meta)
}

fn element(
    raw: &[CacheCounterValue],
    position: usize,
) -> Result<&CacheCounterValue, ResponseDecodeError> {
    raw.get(position)
        .ok_or(ResponseDecodeError::MissingElement { position })
}

fn counter_meta(
    per_counter_meta: &[AtomicCounterMeta],
    index: i64,
) -> Result<&AtomicCounterMeta, ResponseDecodeError> {
    let count = per_counter_meta.len();
    let slot = index
        .checked_sub(1)
        .and_then(|zero_based| usize::try_from(zero_based).ok())
        .ok_or(ResponseDecodeError::CounterIndex { index, count })?;
    per_counter_meta
        .get(slot)
        .ok_or(ResponseDecodeError::CounterIndex { index, count })
}

fn over_limit_response(
    raw: &[CacheCounterValue],
    per_counter_meta: &[AtomicCounterMeta],
) -> Result<RateLimitResponse, ResponseDecodeError> {
    let index = counter_integer("reply[1]", element(raw, 1)?)?;
    let current = counter_integer("reply[2]", element(raw, 2)?)?;
    let limit = counter_integer("reply[3]", element(raw, 3)?)?;
    let meta = counter_meta(per_counter_meta, index)?;
    Ok(RateLimitResponse {
        overall_code: "OVER_LIMIT".to_owned(),
        statuses: vec![RateLimitStatus {
            code: "OVER_LIMIT".to_owned(),
            current_limit: limit,
            limit_remaining: limit.saturating_sub(current).max(0),
            rate_limit_type: status_rate_limit_type(meta.rate_limit_type),
            descriptor_key: meta.descriptor_key.clone(),
            descriptor_value: Some(meta.descriptor_value.clone()),
        }],
        reservation_windows: None,
    })
}

fn success_response(
    raw: &[CacheCounterValue],
    per_counter_meta: &[AtomicCounterMeta],
) -> Result<RateLimitResponse, ResponseDecodeError> {
    let mut statuses = Vec::with_capacity(per_counter_meta.len());
    let mut windows = BTreeSet::new();
    for (index, meta) in per_counter_meta.iter().enumerate() {
        let counter_position = 1 + index * 2;
        let window_position = 2 + index * 2;
        let new_counter = counter_integer(
            &format!("reply[{counter_position}]"),
            element(raw, counter_position)?,
        )?;
        let window_start = counter_integer(
            &format!("reply[{window_position}]"),
            element(raw, window_position)?,
        )?;
        statuses.push(RateLimitStatus {
            code: "OK".to_owned(),
            current_limit: meta.current_limit,
            limit_remaining: meta.current_limit.saturating_sub(new_counter).max(0),
            rate_limit_type: status_rate_limit_type(meta.rate_limit_type),
            descriptor_key: meta.descriptor_key.clone(),
            descriptor_value: Some(meta.descriptor_value.clone()),
        });
        windows.insert(ReservationWindowIdentity {
            counter_key: meta.counter_key.clone(),
            window_start: window_start.to_string(),
            backend: ReservationBackend::Redis,
        });
    }
    Ok(RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses,
        reservation_windows: Some(windows),
    })
}
