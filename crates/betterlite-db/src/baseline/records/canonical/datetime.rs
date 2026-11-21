use chrono::NaiveDateTime;

use crate::autorouter_turn_sql::transaction::TurnTime;

// pydantic's mode="json" datetime: 'Z' for UTC, a numeric offset otherwise, six fraction digits when microseconds are present and none when they are zero.
pub(super) fn turn_time(value: &TurnTime) -> String {
    match value {
        TurnTime::Aware(time) => {
            let mut text = naive(time.naive_local());
            if time.offset().local_minus_utc() == 0 {
                text.push('Z');
            } else {
                text.push_str(&time.offset().to_string());
            }
            text
        }
        TurnTime::Naive(time) => naive(*time),
    }
}

fn naive(value: NaiveDateTime) -> String {
    if value.and_utc().timestamp_subsec_micros() == 0 {
        value.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        value.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
    }
}
