use super::{
    ClassifierInput, DurationSpec, SessionCase, SnapshotSpec, VectorInput, invalid, request_text,
};
use crate::helpers::{HelperError, HelperResult};
use betterlite_logging::spend_bodies::SpendStoragePolicy;
use betterlite_logging::spend_bodies::classifier::{
    SnapshotValue, classifier_audit_fields, classifier_input_snapshot, without_classifier_audit,
};
use betterlite_logging::spend_bodies::placeholder::placeholder_request_body;
use betterlite_logging::spend_bodies::session::{
    SessionInput, SpendTimestamp, batch_trace_session_id, ensure_datetime_utc,
    omits_session_id_when_missing, request_duration_ms, session_id,
};
use betterlite_logging::spend_bodies::vector_store::redact_vector_store;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_logging::spend_sanitize::value::SpendValue;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde_json::{Value, json};
pub(super) fn classifier(call: &ClassifierInput) -> HelperResult {
    let standard = call.standard_logging_object.as_ref();
    let (audit, without) = audit_json(standard)?;
    let storage = SpendStoragePolicy {
        sanitize: SpendSanitizePolicy {
            store_bodies: call.store_bodies,
            max_string_length: call.max_string_length,
        },
        turn_off_message_logging: call.turn_off_message_logging,
    };
    let request = request_text(
        &call.litellm_params,
        standard,
        call.realtime_tools.as_ref(),
        call.dynamic_params.as_ref(),
        storage,
    );
    Ok(json!({
        "request": request,
        "audit": audit,
        "snapshots": snapshots(&call.snapshots)?,
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
            let value = SpendValue::from(&spec.value);
            Ok(
                classifier_input_snapshot(SnapshotValue::Value(&value), spec.openai_sdk)
                    .map_or(Value::Null, |snapshot| json!(snapshot)),
            )
        })
        .collect()
}
fn redacted_vectors(
    vector_store: Option<&Json>,
    store_bodies: bool,
) -> Result<Value, Box<HelperError>> {
    let Some(value) = vector_store else {
        return Ok(Value::Null);
    };
    let Json::List(mut items) = value.clone() else {
        return Err(invalid("vector_store must be a JSON list"));
    };
    redact_vector_store(&mut items, store_bodies);
    Ok(json!(items))
}
pub(super) fn vector_and_placeholder(call: &VectorInput) -> HelperResult {
    let vector_store = redacted_vectors(call.vector_store.as_ref(), call.store_bodies)?;
    Ok(json!({
        "placeholder": placeholder_request_body(&call.litellm_params, &call.model_group, &call.raw_model),
        "vector_store": vector_store,
    }))
}
pub(super) fn session(
    sessions: &[SessionCase],
    durations: &[DurationSpec],
    timestamps: &[String],
) -> HelperResult {
    let values = sessions.iter().map(session_value).collect::<Vec<_>>();
    let duration_values = durations
        .iter()
        .map(|entry| {
            Ok(request_duration_ms(
                timestamp(&entry.start)?,
                timestamp(&entry.end)?,
            ))
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    let instants = timestamps
        .iter()
        .map(|text| {
            let utc = ensure_datetime_utc(timestamp(text)?, &chrono::Utc);
            Ok(utc.map(|moment| moment.timestamp_micros()))
        })
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(json!({
        "sessions": values,
        "durations": duration_values,
        "timestamps": instants,
    }))
}
fn session_value(case: &SessionCase) -> Option<String> {
    let omit = omits_session_id_when_missing(case.metadata.as_ref(), case.configured_omit);
    let batch = batch_trace_session_id(case.call_type.as_deref(), case.request_id.as_deref());
    session_id(&SessionInput {
        kwargs: case.litellm_trace_id.as_ref(),
        metadata: case.metadata.as_ref(),
        standard_logging_payload: case.trace_id.as_ref(),
        omit_when_missing: omit,
        batch_trace_session_id: batch.as_deref(),
        injected_uuid: &case.injected_uuid,
    })
}
fn timestamp(text: &str) -> Result<SpendTimestamp, Box<HelperError>> {
    if let Ok(aware) = DateTime::<FixedOffset>::parse_from_rfc3339(text) {
        return Ok(SpendTimestamp::Aware(aware));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .map(SpendTimestamp::Naive)
        .map_err(|_error| invalid("timestamps must be ISO-8601 text"))
}
