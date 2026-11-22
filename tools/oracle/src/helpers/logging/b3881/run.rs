use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::time::Instant;

use betterlite_error::normalize::class::Class;
use betterlite_logging::prometheus::failure::exception::LabelException;
// ast-grep-ignore: no-long-names
use betterlite_logging::prometheus_deployment::failure::set_llm_deployment_failure_metrics as record_failure_metrics;
use betterlite_logging::prometheus_deployment::fallbacks::{FallbackEvent, log_fallback_event};
use betterlite_logging::prometheus_deployment::requests::{
    FailureRequest, FallbackRequest, SuccessRequest,
};
// ast-grep-ignore: no-long-names
use betterlite_logging::prometheus_deployment::success::set_llm_deployment_success_metrics as record_success_metrics;
use betterlite_models::call_helpers::metadata::MetadataSlots;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use rust_decimal::Decimal;
use serde_json::json;

use super::input::{Function, Input};
use super::probe::Probe;

pub(super) fn run(input: &Input) -> HelperResult {
    let probe = Probe::new();
    match input.function {
        Function::Failure => failure(&probe, input),
        Function::FallbackSuccess => fallback(&probe, input, true)?,
        Function::FallbackFailure => fallback(&probe, input, false)?,
        Function::Success => success(&probe, input),
    };
    Ok(json!({ "events": probe.events_value() }))
}

struct RequestValues {
    model: Option<String>,
    params: JsonObject,
    exception: Option<LabelException>,
    empty_exception: LabelException,
    tags: Vec<String>,
    metadata: MetadataSlots,
    values: UserApiKeyLabelValues,
}

impl RequestValues {
    fn read(input: &Input) -> Self {
        Self {
            model: text(&input.kwargs, "model"),
            params: object_field(&input.kwargs, "litellm_params").unwrap_or_default(),
            exception: input.exception.as_ref().map(probe_exception),
            empty_exception: probe_exception(&BTreeMap::new()),
            tags: tags(&input.kwargs),
            metadata: MetadataSlots {
                metadata: object_field(&input.kwargs, "metadata"),
                litellm_metadata: object_field(&input.kwargs, "litellm_metadata"),
            },
            values: values(input),
        }
    }
}

struct Requests<'parts> {
    failure: FailureRequest<'parts>,
    fallback: FallbackRequest<'parts>,
    success: SuccessRequest<'parts>,
}

fn build_requests<'parts>(parts: &'parts RequestValues, input: &'parts Input) -> Requests<'parts> {
    let now = Instant::now();
    Requests {
        failure: FailureRequest {
            model: parts.model.as_deref(),
            exception: parts.exception.as_ref(),
            litellm_params: &parts.params,
            payload: input.payload.as_ref(),
            catalog: None,
            now,
        },
        fallback: FallbackRequest {
            original_model_group: &input.original_model_group,
            model: parts.model.as_deref(),
            tags: &parts.tags,
            metadata: &parts.metadata,
            exception: parts.exception.as_ref().unwrap_or(&parts.empty_exception),
            catalog: None,
            now,
        },
        success: success_request(parts, input, now),
    }
}

fn success_request<'parts>(
    parts: &'parts RequestValues,
    input: &'parts Input,
    now: Instant,
) -> SuccessRequest<'parts> {
    let completion_start_time = input
        .completion_start_time
        .as_deref()
        .and_then(|text| Decimal::from_str(text).ok());
    let start_time = input
        .start_time
        .as_deref()
        .and_then(|text| Decimal::from_str(text).ok())
        .unwrap_or_default();
    let end_time = input
        .end_time
        .as_deref()
        .and_then(|text| Decimal::from_str(text).ok())
        .unwrap_or_default();
    let output_tokens = input
        .output_tokens
        .as_deref()
        .and_then(|text| Decimal::from_str(text).ok());
    SuccessRequest {
        model: parts.model.as_deref(),
        stream: stream_flag(&input.kwargs),
        completion_start_time,
        litellm_params: &parts.params,
        payload: input.payload.as_ref(),
        exception: parts.exception.as_ref(),
        values: &parts.values,
        context: None,
        start_time,
        end_time,
        output_tokens,
        now,
    }
}

fn failure(probe: &Probe, input: &Input) {
    let parts = RequestValues::read(input);
    let built = build_requests(&parts, input);
    record_failure_metrics(probe, &built.failure);
}

fn fallback(probe: &Probe, input: &Input, success: bool) -> Result<(), Box<HelperError>> {
    let parts = RequestValues::read(input);
    let built = build_requests(&parts, input);
    let event = if success {
        FallbackEvent::Success
    } else {
        FallbackEvent::Failure
    };
    log_fallback_event(probe, &built.fallback, event).or_value_error()?;
    Ok(())
}

fn success(probe: &Probe, input: &Input) {
    let parts = RequestValues::read(input);
    let built = build_requests(&parts, input);
    record_success_metrics(probe, &built.success);
}

fn stream_flag(kwargs: &JsonObject) -> Option<bool> {
    match kwargs.get("stream") {
        Some(Json::Bool(flag)) => Some(*flag),
        Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn probe_exception(fields: &BTreeMap<String, String>) -> LabelException {
    LabelException {
        class_name: "ProbeError".to_owned(),
        class: Class::Other,
        message: None,
        status_code: fields.get("status_code").cloned(),
        code: fields.get("code").cloned(),
        llm_provider: fields.get("llm_provider").cloned(),
        category: None,
        rate_limit_type: None,
        legacy_class_name: None,
    }
}

fn values(input: &Input) -> UserApiKeyLabelValues {
    let entries: Vec<(String, LabelDumpValue)> = input
        .values
        .iter()
        .map(|(name, value)| (name.clone(), LabelDumpValue::String(value.clone())))
        .collect();
    UserApiKeyLabelValues::from_entries(&entries)
}

fn text(kwargs: &JsonObject, key: &str) -> Option<String> {
    match kwargs.get(key) {
        Some(Json::Text(value)) => Some(value.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn object_field(kwargs: &JsonObject, key: &str) -> Option<JsonObject> {
    match kwargs.get(key) {
        Some(Json::Object(entries)) => Some(entries.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

fn tags(kwargs: &JsonObject) -> Vec<String> {
    match kwargs.get("tags") {
        Some(Json::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                Json::Text(text) => Some(text.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => Vec::new(),
    }
}
