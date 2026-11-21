use std::str;

use chrono::{DateTime, FixedOffset, NaiveDateTime, Timelike};

use crate::window_queue::stamp::{WindowStamp, decode_window_stamp};

pub const PARTITION_BOUND_MAX_BYTES: usize = 64 * 1024;

const DEFAULT_MARKER: &str = "DEFAULT";
const UPPER_BOUND_PREFIX: &str = "TO ('";
const UPPER_BOUND_SUFFIX: &str = "')";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionBound {
    Unknown,
    Naive(NaiveDateTime),
    Aware(DateTime<FixedOffset>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPartition {
    pub name: String,
    pub bound: PartitionBound,
}

#[derive(Debug, thiserror::Error)]
pub enum PartitionBoundError {
    #[error(
        "the upper bound {upper} carries a UTC offset and cannot be compared with the naive cutoff {cutoff}"
    )]
    OffsetBound {
        upper: DateTime<FixedOffset>,
        cutoff: NaiveDateTime,
    },
}

// parse_partition_upper_bound: any DEFAULT spelling is kept, and only the pinned TO ('...') form is read
#[must_use]
pub fn parse_partition_upper_bound(expression: &str) -> PartitionBound {
    if expression.to_ascii_uppercase().contains(DEFAULT_MARKER) {
        return PartitionBound::Unknown;
    }
    let Some(pinned) = pinned_upper_bound(expression) else {
        return PartitionBound::Unknown;
    };
    match decode_window_stamp(pinned.as_bytes()) {
        Ok(WindowStamp::Naive(value)) => {
            truncate_microseconds(&value).map_or(PartitionBound::Unknown, PartitionBound::Naive)
        }
        Ok(WindowStamp::Aware(value)) => {
            truncate_microseconds(&value).map_or(PartitionBound::Unknown, PartitionBound::Aware)
        }
        Err(_unparsable) => PartitionBound::Unknown,
    }
}

// Python's datetime keeps microseconds, so a finer fraction names the same bound it compares
fn truncate_microseconds<Stamp>(stamp: &Stamp) -> Option<Stamp>
where
    Stamp: Timelike,
{
    let micros = stamp.nanosecond().checked_div(1_000)?;
    let nanos = micros.checked_mul(1_000)?;
    stamp.with_nanosecond(nanos)
}

// Python's re.search skips a TO ('...') block that is not followed by '), so the first well-formed one wins
fn pinned_upper_bound(expression: &str) -> Option<&str> {
    let mut searched = 0;
    while let Some(offset) = expression.get(searched..)?.find(UPPER_BOUND_PREFIX) {
        let start = searched
            .checked_add(offset)?
            .checked_add(UPPER_BOUND_PREFIX.len())?;
        let remainder = expression.get(start..)?;
        if let Some(end) = remainder.find('\'') {
            let upper = remainder.get(..end)?;
            let suffix = remainder.get(end..)?;
            if suffix.starts_with(UPPER_BOUND_SUFFIX) && !upper.is_empty() {
                return Some(upper);
            }
        }
        searched = start;
    }
    None
}

// decode_partition_bound: the same parse over untrusted bytes, with a ceiling so no allocation follows a catalog value
#[must_use]
pub fn decode_partition_bound(bytes: &[u8]) -> PartitionBound {
    if bytes.len() > PARTITION_BOUND_MAX_BYTES {
        return PartitionBound::Unknown;
    }
    match str::from_utf8(bytes) {
        Ok(expression) => parse_partition_upper_bound(expression),
        Err(_not_text) => PartitionBound::Unknown,
    }
}

// select_partitions_to_drop: an unknown bound, DEFAULT included, is never dropped
pub fn select_partitions_to_drop(
    partitions: &[CatalogPartition],
    cutoff: NaiveDateTime,
) -> Result<Vec<String>, PartitionBoundError> {
    let mut selected = Vec::new();
    for partition in partitions {
        match partition.bound {
            PartitionBound::Unknown => {}
            PartitionBound::Naive(upper) => {
                if upper <= cutoff {
                    selected.push(partition.name.clone());
                }
            }
            PartitionBound::Aware(upper) => {
                return Err(PartitionBoundError::OffsetBound { upper, cutoff });
            }
        }
    }
    Ok(selected)
}
