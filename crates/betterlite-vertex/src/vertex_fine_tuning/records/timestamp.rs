use jiff::Timestamp;
use jiff::civil;

use crate::provider_metadata::timestamp::{LocalTimeContext, decode_vertex_timestamp};

const NANOSECONDS_PER_MICROSECOND: i32 = 1_000;

#[must_use]
pub fn created_at(create_time: Option<&str>, local: &LocalTimeContext) -> i64 {
    let Some(text) = create_time else {
        return 0;
    };
    // LiteLLM replaces every Z with +00:00 before datetime.fromisoformat
    let replaced = text.replace('Z', "+00:00");
    if let Ok(timestamp) = replaced.parse::<Timestamp>() {
        // int() truncates toward zero, while a jiff timestamp floors a pre-epoch instant
        let seconds = timestamp.as_second();
        return if seconds < 0 && timestamp.subsec_nanosecond() > 0 {
            seconds.saturating_add(1)
        } else {
            seconds
        };
    }
    local_created_at(&replaced, local).unwrap_or(0)
}

fn local_created_at(text: &str, local: &LocalTimeContext) -> Option<i64> {
    // an offset that reaches here is one jiff's instant range refused, so it cannot be read as local time
    if text
        .split_once(['T', 't', ' '])
        .is_some_and(|(_date, rest)| rest.contains(['+', '-']))
    {
        return None;
    }
    let datetime = text
        .parse::<civil::DateTime>()
        .or_else(|_error| {
            text.parse::<civil::Date>()
                .map(|date| date.to_datetime(civil::Time::midnight()))
        })
        .ok()?;
    // CPython keeps microseconds, and the shared decoder applies the injected zone the way mktime does
    let micros = datetime
        .subsec_nanosecond()
        .checked_div(NANOSECONDS_PER_MICROSECOND)?;
    let formatted = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{micros:06}Z",
        datetime.year(),
        datetime.month(),
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
        datetime.second(),
    );
    decode_vertex_timestamp(formatted.as_bytes(), local).ok()
}
