use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde_json::Number;

use crate::logging_failure::events::payload_to_map;
use crate::logging_failure::{
    FAILED_API_CALL, FailureCall, FailureDeps, FailureError, FailurePayloadInput, FailureReport,
    FailureTiming,
};

pub fn failure_handler_helper_fn(
    call: &mut FailureCall<'_>,
    report: &FailureReport,
    deps: &FailureDeps<'_>,
) -> Result<FailureTiming, FailureError> {
    let start_time = report.start_time.unwrap_or(call.state.start_time);
    let end_time = report.end_time.unwrap_or_else(|| deps.clock.now());
    if call
        .record
        .exception
        .as_ref()
        .is_some_and(|stored| Arc::ptr_eq(stored, &report.exception))
        && call.record.payload.is_some()
    {
        return Ok(FailureTiming {
            start_time,
            end_time: call.record.end_time.unwrap_or(end_time),
        });
    }
    let traceback = deps.redactor.redact_string(&report.traceback);
    let error_str = deps.redactor.redact_string(&report.error_text());
    record_failure_details(call, &traceback, end_time);
    merge_exception_headers(call, report)?;
    let built = deps.payloads.build(FailurePayloadInput {
        state: call.state,
        exception: &report.exception,
        start_time,
        end_time,
        error_str: &error_str,
    });
    let stored = match &built {
        Some(found) => Json::Object(payload_to_map(found)?),
        None => Json::Null,
    };
    call.state
        .model_call_details
        .insert("standard_logging_object".to_owned(), stored);
    call.record.exception = Some(Arc::clone(&report.exception));
    call.record.end_time = Some(end_time);
    call.record.payload = built.map(Arc::new);
    Ok(FailureTiming {
        start_time,
        end_time,
    })
}

fn record_failure_details(
    call: &mut FailureCall<'_>,
    traceback: &str,
    end_time: DateTime<FixedOffset>,
) {
    let details = &mut call.state.model_call_details;
    details.insert(
        "log_event_type".to_owned(),
        Json::Text(FAILED_API_CALL.to_owned()),
    );
    details.insert(
        "traceback_exception".to_owned(),
        Json::Text(traceback.to_owned()),
    );
    details.insert("end_time".to_owned(), Json::Text(end_time.to_rfc3339()));
    details
        .entry("original_response".to_owned())
        .or_insert(Json::Null);
    if matches!(
        details.get("combined_usage_object"),
        None | Some(Json::Null)
    ) {
        details.insert(
            "response_cost".to_owned(),
            Json::Number(Number::from(0_i64)),
        );
    }
}

fn merge_exception_headers(
    call: &mut FailureCall<'_>,
    report: &FailureReport,
) -> Result<(), FailureError> {
    let Some(headers) = &report.exception.headers else {
        return Ok(());
    };
    let details = &mut call.state.model_call_details;
    let raw_params = details
        .entry("litellm_params".to_owned())
        .or_insert_with(|| Json::Object(IndexMap::new()));
    let Json::Object(params) = raw_params else {
        return Err(FailureError::LitellmParamsNotObject);
    };
    let metadata = match params.get_mut("metadata") {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::Object(metadata)) => metadata,
        Some(_) => return Err(FailureError::MetadataNotObject),
    };
    for (name, value) in headers {
        metadata.insert(name.clone(), Json::Text(value.clone()));
    }
    Ok(())
}
