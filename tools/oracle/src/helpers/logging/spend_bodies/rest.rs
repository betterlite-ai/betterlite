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
use indexmap::IndexMap;
use serde_json::{Value, json};
pub(super) fn classifier(call: &ClassifierInput) -> HelperResult {
    let standard = call.standard_logging_object.as_ref();
    let (audit, without) = match standard {
        None => (Value::Null, Value::Null),
        Some(Json::Object(fields)) => (
            json!(classifier_audit_fields(fields)),
            json!(without_classifier_audit(fields)),
        ),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(invalid("the payload must be an object"));
        }
    };
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
pub(super) fn vector_and_placeholder(call: &VectorInput) -> HelperResult {
    let vector_store = match call.vector_store.as_ref() {
        None => Value::Null,
        Some(Json::List(items)) => {
            let mut redacted = items.clone();
            redact_vector_store(&mut redacted, call.store_bodies);
            json!(redacted)
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(invalid("vector_store must be a JSON list"));
        }
    };
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
    let payload = object_with("trace_id", case.trace_id.as_ref());
    let kwargs = object_with("litellm_trace_id", case.litellm_trace_id.as_ref());
    session_id(&SessionInput {
        kwargs: kwargs.as_ref(),
        metadata: case.metadata.as_ref(),
        standard_logging_payload: payload.as_ref(),
        omit_when_missing: omit,
        batch_trace_session_id: batch.as_deref(),
        injected_uuid: &case.injected_uuid,
    })
}
fn object_with(key: &str, source: Option<&Json>) -> Option<Json> {
    let value = source?;
    Some(Json::Object(IndexMap::from([(
        key.to_owned(),
        value.clone(),
    )])))
}
fn timestamp(text: &str) -> Result<SpendTimestamp, Box<HelperError>> {
    if let Ok(aware) = DateTime::<FixedOffset>::parse_from_rfc3339(text) {
        return Ok(SpendTimestamp::Aware(aware));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .map(SpendTimestamp::Naive)
        .map_err(|_error| invalid("timestamps must be ISO-8601 text"))
}
