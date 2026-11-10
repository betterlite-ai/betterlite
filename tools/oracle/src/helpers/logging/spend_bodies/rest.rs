use super::{Input, SessionCase, SnapshotSpec, policy_of, request_text};
use super::{decode, decode_optional, invalid, required};
use crate::helpers::{HelperError, HelperResult};
use betterlite_logging::spend_bodies::classifier::{
    SnapshotValue, classifier_audit_fields, classifier_input_snapshot, without_classifier_audit,
};
use betterlite_logging::spend_bodies::placeholder::placeholder_request_body;
use betterlite_logging::spend_bodies::session::{
    SessionInput, SpendTimestamp, batch_trace_session_id, ensure_datetime_utc,
    omits_session_id_when_missing, request_duration_ms, session_id,
};
use betterlite_logging::spend_bodies::vector_store::redact_vector_store;
use betterlite_logging::spend_sanitize::value::SpendValue;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde_json::{Value, json};
pub(super) fn classifier(input: &Input) -> HelperResult {
    let standard = decode_optional(input.standard_logging_object.as_ref())?;
    let (audit, without) = audit_json(standard.as_ref())?;
    let snapshots = snapshots(required(input.snapshots.as_ref(), "snapshots")?)?;
    let request = request_text(input, policy_of(input)?)?;
    Ok(json!({
        "request": request,
        "audit": audit,
        "snapshots": snapshots,
        "without": without,
    }))
}
fn audit_json(standard: Option<&Json>) -> Result<(Value, Value), Box<HelperError>> {
    let Some(payload) = standard else {
        return Ok((Value::Null, Value::Null));
    };
    let Json::Object(fields) = payload else {
        return Err(invalid("the payload must be an object"));
    };
    Ok((
        json!(classifier_audit_fields(fields)),
        json!(without_classifier_audit(fields)),
    ))
}
fn snapshots(specs: &[SnapshotSpec]) -> Result<Vec<Value>, Box<HelperError>> {
    specs
        .iter()
        .map(|spec| {
            let decoded = decode(&spec.value)?;
            let value = SpendValue::from(&decoded);
            Ok(
                classifier_input_snapshot(SnapshotValue::Value(&value), spec.openai_sdk)
                    .map_or(Value::Null, |snapshot| json!(snapshot)),
            )
        })
        .collect()
}
fn redacted_vectors(input: &Input, store_bodies: bool) -> Result<Value, Box<HelperError>> {
    let Some(value) = input.vector_store.as_ref() else {
        return Ok(Value::Null);
    };
    let Json::List(mut items) = decode(value)? else {
        return Err(invalid("vector_store must be a JSON list"));
    };
    redact_vector_store(&mut items, store_bodies);
    Ok(json!(items))
}
pub(super) fn vector_and_placeholder(input: &Input) -> HelperResult {
    let store_bodies = required(input.store_bodies, "store_bodies")?;
    let params = decode(required(input.litellm_params.as_ref(), "litellm_params")?)?;
    let model_group = required(input.model_group.as_deref(), "model_group")?;
    let raw_model = required(input.raw_model.as_deref(), "raw_model")?;
    let vector_store = redacted_vectors(input, store_bodies)?;
    Ok(json!({
        "placeholder": placeholder_request_body(&params, model_group, raw_model),
        "vector_store": vector_store,
    }))
}
pub(super) fn session(input: &Input) -> HelperResult {
    let values = required(input.sessions.as_ref(), "sessions")?
        .iter()
        .map(session_value)
        .collect::<Result<Vec<_>, _>>()?;
    let durations = required(input.durations.as_ref(), "durations")?
        .iter()
        .map(|entry| {
            Ok(request_duration_ms(
                timestamp(&entry.start)?,
                timestamp(&entry.end)?,
            ))
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    let instants = required(input.timestamps.as_ref(), "timestamps")?
        .iter()
        .map(|text| {
            let utc = ensure_datetime_utc(timestamp(text)?, &chrono::Utc);
            Ok(utc.map(|moment| moment.timestamp_micros()))
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(json!({
        "sessions": values,
        "durations": durations,
        "timestamps": instants,
    }))
}
fn session_value(case: &SessionCase) -> Result<Option<String>, Box<HelperError>> {
    let metadata = decode_optional(case.metadata.as_ref())?;
    let trace = decode_optional(case.trace_id.as_ref())?;
    let kwargs = decode_optional(case.litellm_trace_id.as_ref())?;
    let omit = omits_session_id_when_missing(metadata.as_ref(), case.configured_omit);
    let batch = batch_trace_session_id(case.call_type.as_deref(), case.request_id.as_deref());
    Ok(session_id(&SessionInput {
        kwargs: kwargs.as_ref(),
        metadata: metadata.as_ref(),
        standard_logging_payload: trace.as_ref(),
        omit_when_missing: omit,
        batch_trace_session_id: batch.as_deref(),
        injected_uuid: &case.injected_uuid,
    }))
}
fn timestamp(text: &str) -> Result<SpendTimestamp, Box<HelperError>> {
    if let Ok(aware) = DateTime::<FixedOffset>::parse_from_rfc3339(text) {
        return Ok(SpendTimestamp::Aware(aware));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .map(SpendTimestamp::Naive)
        .map_err(|_error| invalid("timestamps must be ISO-8601 text"))
}
