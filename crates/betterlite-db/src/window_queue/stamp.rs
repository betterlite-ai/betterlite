use std::str;

use chrono::{
    DateTime, Duration, FixedOffset, Local, LocalResult, NaiveDate, NaiveDateTime, NaiveTime,
    Offset as _, TimeZone as _, Utc,
};
use rust_decimal::Decimal;

use super::{WindowQueueError, WindowSpendTransaction};
use crate::spend_queues::entity::SpendAmount;

// Python's isoformat(timespec="microseconds"), the shape the queue stores and the Redis buffer carries
const STAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.6f";

// Python's str(datetime) writes a space before the offset, and a spend log read back from the database carries that shape
const OFFSET_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f%#z";

const NAIVE_FORMATS: [&str; 4] = [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M",
];

const DATE_FORMAT: &str = "%Y-%m-%d";

const PROBE_STEP_MINUTES: i64 = 15;
// no zone has skipped more than a day, so 26 hours of quarter-hour probes reaches the offset before any gap
const PROBE_STEPS: i64 = 104;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowStamp {
    Naive(NaiveDateTime),
    Aware(DateTime<FixedOffset>),
}

// What the host's local zone says about a wall-clock time: Python's astimezone reads it through libc, which resolves an overlap to its first occurrence and a nonexistent time to the offset in effect before the jump.
pub trait LocalZone: Send + Sync {
    fn offset_for(&self, local: NaiveDateTime) -> FixedOffset;
}

pub struct SystemZone;

impl LocalZone for SystemZone {
    // The host zone is fixed when the process starts, so no test can reach a non-UTC branch here; tests inject a FixedZone.
    #[cfg_attr(test, mutants::skip)]
    fn offset_for(&self, local: NaiveDateTime) -> FixedOffset {
        match Local.from_local_datetime(&local) {
            LocalResult::Single(instant) => *instant.offset(),
            LocalResult::Ambiguous(earlier, _later) => *earlier.offset(),
            LocalResult::None => offset_before_gap(local),
        }
    }
}

// libc reads a wall-clock time inside a spring-forward gap with the offset before the jump, so walk back until the zone answers again.
#[cfg_attr(test, mutants::skip)]
fn offset_before_gap(local: NaiveDateTime) -> FixedOffset {
    let step = Duration::minutes(PROBE_STEP_MINUTES);
    let mut probe = local;
    for _ in 0_i64..PROBE_STEPS {
        let Some(earlier) = probe.checked_sub_signed(step) else {
            break;
        };
        probe = earlier;
        match Local.from_local_datetime(&probe) {
            LocalResult::Single(instant) => return *instant.offset(),
            LocalResult::Ambiguous(earlier_instant, _later) => return *earlier_instant.offset(),
            LocalResult::None => {}
        }
    }
    local.and_utc().offset().fix()
}

// build_window_spend_transaction: the six fields of one queued increment, with the caller's stamps kept as timestamps rather than text.
pub struct WindowIncrement<'text> {
    pub entity_type: &'text str,
    pub entity_id: &'text str,
    pub window_duration: &'text str,
    pub window_start: WindowStamp,
    pub spend: Decimal,
    pub started_at: Option<WindowStamp>,
}

pub fn build_window_spend_transaction(
    increment: &WindowIncrement<'_>,
    zone: &dyn LocalZone,
) -> Result<WindowSpendTransaction, WindowQueueError> {
    let window_start = match increment.window_start {
        WindowStamp::Naive(local) => local,
        WindowStamp::Aware(aware) => aware.with_timezone(&Utc).naive_utc(),
    };
    let started_at = increment
        .started_at
        .map(|stamp| {
            started_at_utc(stamp, zone).map(|naive| naive.format(STAMP_FORMAT).to_string())
        })
        .transpose()?;
    Ok(WindowSpendTransaction {
        entity_type: increment.entity_type.to_owned(),
        entity_id: increment.entity_id.to_owned(),
        window_duration: increment.window_duration.to_owned(),
        window_start: window_start.format(STAMP_FORMAT).to_string(),
        spend: SpendAmount(increment.spend),
        started_at,
    })
}

// to_naive_utc: an aware value converts to UTC, while a naive one is first read in the host zone, as astimezone(timezone.utc) does.
fn started_at_utc(
    stamp: WindowStamp,
    zone: &dyn LocalZone,
) -> Result<NaiveDateTime, WindowQueueError> {
    match stamp {
        WindowStamp::Naive(local) => local
            .checked_sub_signed(Duration::seconds(i64::from(
                zone.offset_for(local).local_minus_utc(),
            )))
            .ok_or(WindowQueueError::StampOutOfRange),
        WindowStamp::Aware(aware) => Ok(aware.with_timezone(&Utc).naive_utc()),
    }
}

// datetime.fromisoformat's shapes: an offset wins, then the naive forms, then a bare date at midnight.
pub fn decode_window_stamp(text: &[u8]) -> Result<WindowStamp, WindowQueueError> {
    let value =
        str::from_utf8(text).map_err(|source| WindowQueueError::InvalidStampText { source })?;
    let rfc_error = match DateTime::parse_from_rfc3339(value) {
        Ok(aware) => return Ok(WindowStamp::Aware(aware)),
        Err(error) => error,
    };
    if let Ok(aware) = DateTime::parse_from_str(value, OFFSET_FORMAT) {
        return Ok(WindowStamp::Aware(aware));
    }
    for format in NAIVE_FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(WindowStamp::Naive(naive));
        }
    }
    match NaiveDate::parse_from_str(value, DATE_FORMAT) {
        Ok(date) => Ok(WindowStamp::Naive(date.and_time(NaiveTime::MIN))),
        Err(_unparsable) => Err(WindowQueueError::InvalidStamp { source: rfc_error }),
    }
}
