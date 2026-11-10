use chrono::{DateTime, FixedOffset, LocalResult, NaiveDateTime, TimeZone, Utc};

use super::support::{counts_as_present, string_value};
use crate::spend_sanitize::value::{SpendMapping, SpendValue};

pub const SESSION_ID_OMITTED_KEY: &str = "litellm_session_id_omitted";
pub const BATCH_COST_SUFFIX: &str = "_batch_cost";

const BATCH_CALL_TYPES: [&str; 4] = [
    "create_batch",
    "acreate_batch",
    "retrieve_batch",
    "aretrieve_batch",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpendTimestamp {
    Aware(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

pub struct SessionInput<'data> {
    pub kwargs: Option<&'data SpendMapping>,
    pub metadata: Option<&'data SpendMapping>,
    pub standard_logging_payload: Option<&'data SpendMapping>,
    pub omit_when_missing: bool,
    pub batch_trace_session_id: Option<&'data str>,
    pub injected_uuid: &'data str,
}

#[must_use]
pub fn omits_session_id_when_missing(
    metadata: Option<&SpendMapping>,
    configured_omit: bool,
) -> bool {
    if configured_omit {
        return true;
    }
    metadata
        .and_then(|bucket| bucket.entries.get(SESSION_ID_OMITTED_KEY))
        .is_some_and(counts_as_present)
}

#[must_use]
pub fn batch_trace_session_id(call_type: Option<&str>, request_id: Option<&str>) -> Option<String> {
    let batch_call = call_type?;
    if !BATCH_CALL_TYPES.contains(&batch_call) {
        return None;
    }
    let batch_request = request_id.filter(|value| !value.is_empty())?;
    Some(
        batch_request
            .strip_suffix(BATCH_COST_SUFFIX)
            .unwrap_or(batch_request)
            .to_owned(),
    )
}

#[must_use]
pub fn session_id(input: &SessionInput<'_>) -> Option<String> {
    if input.omit_when_missing {
        let session = input
            .metadata
            .and_then(|bucket| bucket.entries.get("session_id"));
        return session
            .filter(|value| counts_as_present(value))
            .map(string_value);
    }
    if let Some(batch) = input.batch_trace_session_id {
        return Some(batch.to_owned());
    }
    if let Some(payload) = input.standard_logging_payload
        && let Some(trace) = payload.entries.get("trace_id")
        && !matches!(trace, SpendValue::Null)
    {
        return Some(string_value(trace));
    }
    if let Some(kwargs) = input.kwargs
        && let Some(trace) = kwargs.entries.get("litellm_trace_id")
        && !matches!(trace, SpendValue::Null)
    {
        return Some(string_value(trace));
    }
    Some(input.injected_uuid.to_owned())
}

#[must_use]
pub fn request_duration_ms(start: SpendTimestamp, end: SpendTimestamp) -> Option<i64> {
    let delta = match (start, end) {
        (SpendTimestamp::Aware(from), SpendTimestamp::Aware(to)) => to.signed_duration_since(from),
        (SpendTimestamp::Naive(from), SpendTimestamp::Naive(to)) => to.signed_duration_since(from),
        (SpendTimestamp::Aware(_), SpendTimestamp::Naive(_))
        | (SpendTimestamp::Naive(_), SpendTimestamp::Aware(_)) => return None,
    };
    let micros = delta.num_microseconds()?;
    micros.checked_div(1_000)
}

#[must_use]
pub fn ensure_datetime_utc<Tz>(timestamp: SpendTimestamp, local: &Tz) -> Option<DateTime<Utc>>
where
    Tz: TimeZone,
{
    match timestamp {
        SpendTimestamp::Aware(datetime) => Some(datetime.with_timezone(&Utc)),
        SpendTimestamp::Naive(naive) => match local.from_local_datetime(&naive) {
            LocalResult::Single(datetime) => Some(datetime.with_timezone(&Utc)),
            LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
            LocalResult::None => None,
        },
    }
}
