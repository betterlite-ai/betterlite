use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::datadog::apm::{tag_call_id, tag_request};
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::openai_base_config::Object as JsonMap;
use chrono::{DateTime, FixedOffset};

use super::logging::{LoggingRequest, build_state};
use super::{PreCallError, PreCallInput, PreCallStep, internal};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::rewrites::{resolve_call_id, stream_usage_updates, streamable_models};
use crate::pre_call::route_type::RouteType;

pub(super) fn identify<K: PreCallKey>(
    step: &PreCallStep<K>,
    data: &mut JsonMap,
    input: &PreCallInput<'_, K>,
) -> String {
    let call_id = resolve_call_id(
        input
            .request
            .headers
            .get("x-litellm-call-id")
            .and_then(|header| header.to_str().ok()),
    );
    data.insert(
        String::from("litellm_call_id"),
        JsonValue::String(call_id.clone()),
    );
    tag_call(step, data, call_id.as_str());
    call_id
}

fn tag_call<K: PreCallKey>(step: &PreCallStep<K>, data: &JsonMap, call_id: &str) {
    let tracer = step.ports.tracer.as_ref();
    tag_call_id(tracer, Some(call_id));
    tag_request(
        tracer,
        None,
        None,
        data.get("model").and_then(JsonValue::as_str),
    );
}

pub(super) fn stream_usage<K: PreCallKey>(
    step: &PreCallStep<K>,
    data: &mut JsonMap,
    route_type: RouteType,
    team_id: Option<&str>,
) {
    let target_model = data.get("model").cloned().unwrap_or(JsonValue::Null);
    let updates = stream_usage_updates(
        data,
        step.settings.pre_call.always_include_stream_usage,
        route_type,
        || {
            streamable_models(
                &target_model,
                step.ports.router.as_deref(),
                team_id,
                step.ports.stream_options.as_ref(),
            )
        },
    );
    data.extend(updates);
}

pub(super) fn build_logging<K: PreCallKey>(
    step: &PreCallStep<K>,
    data: &mut JsonMap,
    input: &PreCallInput<'_, K>,
    call_id: &str,
    start_time: DateTime<FixedOffset>,
) -> Result<LoggingState, PreCallError> {
    let redactor = step.redactor.as_ref().map_err(|error| PreCallError {
        failure: internal(error.to_string()),
        data: data.clone(),
    })?;
    let held = data.clone();
    build_state(LoggingRequest {
        data,
        route_type: input.route_type,
        start_time,
        call_id,
        loggers: step.loggers.as_ref(),
        redactor,
    })
    .map_err(|failure| PreCallError {
        failure,
        data: held,
    })
}
