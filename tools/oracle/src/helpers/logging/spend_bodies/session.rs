use betterlite_logging::spend_bodies::session::{
    SessionInput, batch_trace_session_id, ensure_datetime_utc, omits_session_id_when_missing,
    request_duration_ms, session_id,
};
use betterlite_logging::spend_bodies::support::NodeAllocator;
use serde_json::{Value, json};

use super::DurationSpec;
use super::pipeline::{optional_decode, timestamp};
use super::support::mapping_of;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionSpec {
    sessions: Vec<SessionCase>,
    durations: Vec<DurationSpec>,
    timestamps: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionCase {
    metadata: Option<Value>,
    configured_omit: bool,
    call_type: Option<String>,
    request_id: Option<String>,
    trace_id: Option<Value>,
    litellm_trace_id: Option<Value>,
    injected_uuid: String,
}

pub(super) fn session(spec: &SessionSpec) -> crate::helpers::HelperResult {
    let mut values = Vec::with_capacity(spec.sessions.len());
    for case in &spec.sessions {
        values.push(session_value(case)?);
    }
    let mut durations = Vec::with_capacity(spec.durations.len());
    for entry in &spec.durations {
        durations.push(request_duration_ms(
            timestamp(&entry.start)?,
            timestamp(&entry.end)?,
        ));
    }
    let mut instants = Vec::with_capacity(spec.timestamps.len());
    for text in &spec.timestamps {
        let moment = timestamp(text)?;
        instants.push(ensure_datetime_utc(moment, &chrono::Utc).map(|utc| utc.timestamp_micros()));
    }
    Ok(json!({
        "sessions": values,
        "durations": durations,
        "timestamps": instants,
    }))
}

fn session_value(case: &SessionCase) -> crate::helpers::HelperResult {
    let mut nodes = NodeAllocator::default();
    let stored_metadata = optional_decode(case.metadata.as_ref(), &mut nodes)?;
    let metadata = stored_metadata
        .as_ref()
        .and_then(|value| mapping_of(value).ok());
    let trace = optional_decode(case.trace_id.as_ref(), &mut nodes)?;
    let stored_kwargs = optional_decode(case.litellm_trace_id.as_ref(), &mut nodes)?;
    let omit = omits_session_id_when_missing(metadata, case.configured_omit);
    let batch = batch_trace_session_id(case.call_type.as_deref(), case.request_id.as_deref());
    let value = session_id(&SessionInput {
        kwargs: stored_kwargs
            .as_ref()
            .and_then(|value| mapping_of(value).ok()),
        metadata,
        standard_logging_payload: trace.as_ref().and_then(|value| mapping_of(value).ok()),
        omit_when_missing: omit,
        batch_trace_session_id: batch.as_deref(),
        injected_uuid: &case.injected_uuid,
    });
    Ok(value.map_or(Value::Null, Value::String))
}
