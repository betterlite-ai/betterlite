use betterlite_logging::spend_bodies::classifier::{
    SnapshotValue, classifier_audit_fields, classifier_input_snapshot, without_classifier_audit,
};
use betterlite_logging::spend_bodies::placeholder::placeholder_request_body;
use betterlite_logging::spend_bodies::session::{
    SessionInput, SpendTimestamp, batch_trace_session_id, ensure_datetime_utc,
    omits_session_id_when_missing, request_duration_ms, session_id,
};
use betterlite_logging::spend_bodies::support::Fields;
use betterlite_logging::spend_bodies::vector_store::redact_vector_store;
use betterlite_logging::spend_sanitize::value::SpendValue;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use serde_json::{Value, json};

use super::support::{decode, decode_optional, invalid, required};
use super::{Input, SessionCase, SnapshotSpec, policy_of, request_text};

pub(super) fn classifier(input: &Input) -> crate::helpers::HelperResult {
    let standard = decode_optional(input.standard_logging_object.as_ref())?;
    let (audit, without) = audit_json(standard.as_ref())?;
    let snapshots = snapshots(required(input.snapshots.as_ref(), "snapshots")?)?;
    let policy = policy_of(input)?;
    let params = decode(required(input.litellm_params.as_ref(), "litellm_params")?)?;
    let tools = decode_optional(input.realtime_tools.as_ref())?;
    let dynamic = decode_optional(input.dynamic_params.as_ref())?;
    let request = request_text(
        &params,
        standard.as_ref(),
        tools.as_ref(),
        dynamic.as_ref(),
        policy,
    );
    Ok(json!({
        "snapshots": snapshots,
        "audit": audit,
        "without": without,
        "request": request,
    }))
}

fn audit_json(standard: Option<&Json>) -> Result<(Value, Value), Box<crate::helpers::HelperError>> {
    let Some(payload) = standard else {
        return Ok((Value::Null, Value::Null));
    };
    let fields = fields_of(payload)?;
    Ok((
        serde_json::to_value(classifier_audit_fields(fields))
            .map_err(|error| invalid(&error.to_string()))?,
        serde_json::to_value(without_classifier_audit(fields))
            .map_err(|error| invalid(&error.to_string()))?,
    ))
}

fn snapshots(specs: &[SnapshotSpec]) -> Result<Vec<Value>, Box<crate::helpers::HelperError>> {
    let mut values = Vec::with_capacity(specs.len());
    for spec in specs {
        let decoded = decode(&spec.value)?;
        let snapshot = classifier_input_snapshot(
            SnapshotValue::Value(&SpendValue::from(&decoded)),
            spec.openai_sdk,
        );
        values.push(match snapshot {
            Some(found) => {
                serde_json::to_value(found).map_err(|error| invalid(&error.to_string()))?
            }
            None => Value::Null,
        });
    }
    Ok(values)
}

fn fields_of(value: &Json) -> Result<&Fields, Box<crate::helpers::HelperError>> {
    let Json::Object(fields) = value else {
        return Err(invalid("the payload must be an object"));
    };
    Ok(fields)
}

pub(super) fn vector_and_placeholder(input: &Input) -> crate::helpers::HelperResult {
    let store_bodies = required(input.store_bodies, "store_bodies")?;
    let params = decode(required(input.litellm_params.as_ref(), "litellm_params")?)?;
    let model_group = required(input.model_group.as_deref(), "model_group")?;
    let raw_model = required(input.raw_model.as_deref(), "raw_model")?;
    let vector_store = match input.vector_store.as_ref() {
        Some(value) => {
            let Json::List(mut items) = decode(value)? else {
                return Err(invalid("vector_store must be a JSON list"));
            };
            redact_vector_store(&mut items, store_bodies);
            json!(items)
        }
        None => Value::Null,
    };
    Ok(json!({
        "vector_store": vector_store,
        "placeholder": placeholder_request_body(&params, model_group, raw_model),
    }))
}

pub(super) fn session(input: &Input) -> crate::helpers::HelperResult {
    let cases = required(input.sessions.as_ref(), "sessions")?;
    let mut values = Vec::with_capacity(cases.len());
    for case in cases {
        values.push(session_value(case)?);
    }
    let durations = required(input.durations.as_ref(), "durations")?;
    let mut duration_values = Vec::with_capacity(durations.len());
    for entry in durations {
        duration_values.push(request_duration_ms(
            timestamp(&entry.start)?,
            timestamp(&entry.end)?,
        ));
    }
    let timestamps = required(input.timestamps.as_ref(), "timestamps")?;
    let mut instants = Vec::with_capacity(timestamps.len());
    for text in timestamps {
        let moment = timestamp(text)?;
        instants.push(ensure_datetime_utc(moment, &chrono::Utc).map(|utc| utc.timestamp_micros()));
    }
    Ok(json!({
        "sessions": values,
        "durations": duration_values,
        "timestamps": instants,
    }))
}

fn session_value(case: &SessionCase) -> Result<Option<String>, Box<crate::helpers::HelperError>> {
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

fn timestamp(text: &str) -> Result<SpendTimestamp, Box<crate::helpers::HelperError>> {
    if let Ok(aware) = DateTime::<FixedOffset>::parse_from_rfc3339(text) {
        return Ok(SpendTimestamp::Aware(aware));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f") {
        return Ok(SpendTimestamp::Naive(naive));
    }
    Err(invalid("timestamps must be ISO-8601 text"))
}
