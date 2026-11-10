use super::support::{as_object, counts_as_present};
use crate::python_json;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
pub const SESSION_ID_OMITTED_KEY: &str = "litellm_session_id_omitted";
pub const BATCH_COST_SUFFIX: &str = "_batch_cost";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpendTimestamp {
    Aware(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}
pub struct SessionInput<'data> {
    pub kwargs: Option<&'data Json>,
    pub metadata: Option<&'data Json>,
    pub standard_logging_payload: Option<&'data Json>,
    pub omit_when_missing: bool,
    pub batch_trace_session_id: Option<&'data str>,
    pub injected_uuid: &'data str,
}
#[must_use]
pub fn omits_session_id_when_missing(metadata: Option<&Json>, configured_omit: bool) -> bool {
    configured_omit
        || metadata.and_then(as_object).is_some_and(|fields| {
            fields
                .get(SESSION_ID_OMITTED_KEY)
                .is_some_and(counts_as_present)
        })
}
#[must_use]
pub fn batch_trace_session_id(call_type: Option<&str>, request_id: Option<&str>) -> Option<String> {
    let batch_call = call_type?;
    let batch = matches!(
        batch_call,
        "create_batch" | "acreate_batch" | "retrieve_batch" | "aretrieve_batch"
    );
    if !batch {
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
        return input
            .metadata
            .and_then(as_object)
            .and_then(|fields| fields.get("session_id"))
            .filter(|value| counts_as_present(value))
            .map(string_value);
    }
    if let Some(batch) = input.batch_trace_session_id {
        return Some(batch.to_owned());
    }
    if let Some(trace) = trace_text(input.standard_logging_payload, "trace_id") {
        return Some(trace);
    }
    if let Some(trace) = trace_text(input.kwargs, "litellm_trace_id") {
        return Some(trace);
    }
    Some(input.injected_uuid.to_owned())
}
// Python's str() for the declared string types, with containers projected as JSON
fn string_value(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null => "None".to_owned(),
        Json::Bool(flag) => (if *flag { "True" } else { "False" }).to_owned(),
        Json::Number(_) | Json::List(_) | Json::Object(_) => python_json::dumps(value),
    }
}
fn trace_text(source: Option<&Json>, key: &str) -> Option<String> {
    source
        .and_then(as_object)
        .and_then(|fields| fields.get(key))
        .filter(|value| !matches!(value, Json::Null))
        .map(string_value)
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
pub fn ensure_datetime_utc<T>(moment: SpendTimestamp, zone: &T) -> Option<DateTime<Utc>>
where
    T: TimeZone,
{
    match moment {
        SpendTimestamp::Aware(datetime) => Some(datetime.with_timezone(&Utc)),
        SpendTimestamp::Naive(naive) => zone
            .from_local_datetime(&naive)
            .earliest()
            .map(|datetime| datetime.with_timezone(&Utc)),
    }
}
