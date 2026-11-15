use std::collections::BTreeSet;

use crate::limiter_records::{
    AtomicCounterMeta, CacheCounterValue, RateLimitResponse, RateLimitStatus, ReservationBackend,
    ReservationWindowIdentity,
};

use super::status::counter_status;
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
        let index = counter_integer("reply[1]", element(raw, 1)?)?;
        let current = counter_integer("reply[2]", element(raw, 2)?)?;
        let limit = counter_integer("reply[3]", element(raw, 3)?)?;
        let meta = counter_meta(per_counter_meta, index)?;
        return Ok(RateLimitResponse {
            overall_code: "OVER_LIMIT".to_owned(),
            statuses: vec![counter_status("OVER_LIMIT", limit, current, meta)],
            reservation_windows: None,
        });
    }
    let (statuses, windows) = success_counters(raw, per_counter_meta)?;
    Ok(RateLimitResponse {
        overall_code: "OK".to_owned(),
        statuses,
        reservation_windows: Some(windows),
    })
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

fn success_counters(
    raw: &[CacheCounterValue],
    per_counter_meta: &[AtomicCounterMeta],
) -> Result<(Vec<RateLimitStatus>, BTreeSet<ReservationWindowIdentity>), ResponseDecodeError> {
    let mut statuses = Vec::with_capacity(per_counter_meta.len());
    let mut windows = BTreeSet::new();
    for (index, meta) in per_counter_meta.iter().enumerate() {
        let counter_position = index.saturating_mul(2).saturating_add(1);
        let window_position = index.saturating_mul(2).saturating_add(2);
        let new_counter = counter_integer(
            &format!("reply[{counter_position}]"),
            element(raw, counter_position)?,
        )?;
        let window_start = counter_integer(
            &format!("reply[{window_position}]"),
            element(raw, window_position)?,
        )?;
        statuses.push(counter_status("OK", meta.current_limit, new_counter, meta));
        windows.insert(ReservationWindowIdentity {
            counter_key: meta.counter_key.clone(),
            window_start: window_start.to_string(),
            backend: ReservationBackend::Redis,
        });
    }
    Ok((statuses, windows))
}
