use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::datadog::apm::{tag_call_id, tag_request};
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::openai_base_config::Object as JsonMap;
use chrono::{DateTime, FixedOffset};

use super::logging::{LoggingRequest, build_state};
use super::{PreCallError, PreCallInput, PreCallStep, internal};
use crate::pre_call::key::PreCallKey;
use crate::pre_call::rewrites::{resolve_call_id, stream_usage_updates, streamable_models};

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
    let tracer = step.ports.tracer.as_ref();
    tag_call_id(tracer, Some(call_id.as_str()));
    tag_request(
        tracer,
        None,
        None,
        data.get("model").and_then(JsonValue::as_str),
    );
    call_id
}

pub(super) fn stream_usage<K: PreCallKey>(
    step: &PreCallStep<K>,
    data: &mut JsonMap,
    input: &PreCallInput<'_, K>,
) {
    let target_model = data.get("model").cloned().unwrap_or(JsonValue::Null);
    let updates = stream_usage_updates(
        data,
        step.settings.pre_call.always_include_stream_usage,
        input.route_type,
        || {
            streamable_models(
                &target_model,
                step.ports.router.as_deref(),
                input.key.team_id(),
                step.ports.stream_options.as_ref(),
            )
        },
    );
    for (name, value) in updates {
        data.insert(name, value);
    }
}

pub(super) fn build_logging<K: PreCallKey>(
    step: &PreCallStep<K>,
    data: &mut JsonMap,
    input: &PreCallInput<'_, K>,
    call_id: &str,
    start_time: DateTime<FixedOffset>,
) -> Result<LoggingState, PreCallError> {
    let Ok(redactor) = step.redactor.as_ref() else {
        let reason = step.redactor.as_ref().err().cloned().unwrap_or_default();
        return Err(PreCallError {
            failure: internal(reason),
            data: data.clone(),
        });
    };
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
