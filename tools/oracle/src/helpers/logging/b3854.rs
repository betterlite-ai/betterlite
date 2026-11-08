use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_policy::annotations::{
    set_service_tier_attributes, set_team_attributes, write_identity_attributes,
    write_team_attributes,
};
use betterlite_logging::otel::legacy_policy::capture::{CapturePolicy, LoggingFlags};
use betterlite_logging::otel::legacy_policy::config::OpenTelemetryConfig;
use betterlite_logging::otel::legacy_policy::dedupe::EmittedSpans;
use betterlite_logging::otel::legacy_policy::error_recording::{
    RecordedException, record_exception_on_span,
};
use betterlite_logging::otel::legacy_policy::semconv_attrs::{
    gen_ai_operation_name, semconv_latest_experimental, set_cache_token_attributes,
    set_semconv_request_attributes,
};
use betterlite_logging::otel::legacy_policy::span_time::{SpanTime, span_name, to_ns};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperResult;

use self::inputs::{Call, ConfigInput, ModelMap, NameInput, TeamInput, TimeInput};
use self::support::{config_json, invalid, metadata_keys, normalized, record};

mod inputs;
mod support;
mod tests;

pub(crate) struct Op;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Input {
    function: String,
    environment: BTreeMap<String, String>,
    config: ConfigInput,
    configs: Vec<ConfigInput>,
    values: Vec<Value>,
    flags: LoggingFlags,
    calls: Vec<Call>,
    times: Vec<TimeInput>,
    now: i64,
    names: Vec<NameInput>,
    teams: Vec<TeamInput>,
    payload: ModelMap,
    litellm_params: ModelMap,
    baggage_team_metadata_keys: Vec<String>,
    baggage_metadata_keys: Vec<String>,
    payloads: Vec<ModelMap>,
    exception: Option<RecordedException>,
    optional_params: ModelMap,
    call_types: Vec<Option<String>>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function.as_str() {
            "config_from_env" => Ok(config_json(&normalized(
                &OpenTelemetryConfig::from_env(&input.environment),
                &input.environment,
            ))),
            "config_normalize" => Ok(config_json(&normalized(
                &input.config.to_config(),
                &input.environment,
            ))),
            "metadata_keys" => Ok(json!(
                input.values.iter().map(metadata_keys).collect::<Vec<_>>()
            )),
            "capture_sample" => Ok(capture_sample(input)),
            "capture_resolve" => Ok(capture_resolve(input)),
            "emit_once" => Ok(emit_once(input)),
            "span_time" => Ok(span_time(input)),
            "team" => team_attributes(input),
            "team_from_payload" => record(|span| write_team_attributes(span, &input.payload)),
            "identity" => identity(input),
            "service_tier" => service_tier(input),
            "exception_stamp" => record(|span| {
                record_exception_on_span(span, input.exception.as_ref(), &input.payload);
            }),
            "semconv_request" => record(|span| {
                set_semconv_request_attributes(span, &input.optional_params);
            }),
            "semconv_cache_and_operation" => cache_and_operation(input),
            "semconv_gate" => Ok(json!(semconv_latest_experimental(
                &input.config.to_config()
            ))),
            other => Err(invalid(&format!("unknown function {other}"))),
        }
    }
}

fn capture_sample(input: &Input) -> Value {
    let mut records = Vec::with_capacity(input.configs.len());
    for config in &input.configs {
        let policy = CapturePolicy::from_init_state(&config.to_config(), &input.environment);
        records.push(json!({
            "without_message_logging": policy.resolve(LoggingFlags::default()).as_str(),
            "with_message_logging": policy.resolve(LoggingFlags { turn_off_message_logging: false, message_logging: true }).as_str(),
        }));
    }
    Value::Array(records)
}

fn capture_resolve(input: &Input) -> Value {
    let policy = CapturePolicy::from_init_state(&input.config.to_config(), &input.environment);
    let mode = policy.resolve(input.flags);
    json!({
        "in_span": mode.capture_in_span(),
        "mode": mode.as_str(),
        "in_event": mode.capture_in_event(),
    })
}

fn emit_once(input: &Input) -> Value {
    let mut spans = EmittedSpans::default();
    let emitted = input
        .calls
        .iter()
        .map(|call: &Call| spans.emit_once(&call.handler, &call.scope));
    json!(emitted.collect::<Vec<_>>())
}

fn span_time(input: &Input) -> Value {
    let nanoseconds = input.times.iter().map(|time| {
        let span = match time {
            TimeInput::Seconds(seconds) => SpanTime::Seconds(*seconds),
            TimeInput::DateTime(datetime) => SpanTime::DateTime(*datetime),
        };
        to_ns(span, input.now)
    });
    let names = input.names.iter().map(|name| {
        span_name(
            &name.litellm_params,
            gen_ai_operation_name(name.call_type.as_deref()),
            name.model.as_deref(),
            name.semconv,
        )
    });
    json!({
        "ns": nanoseconds.collect::<Vec<_>>(),
        "names": names.collect::<Vec<_>>(),
    })
}

fn team_attributes(input: &Input) -> HelperResult {
    input
        .teams
        .iter()
        .map(|team| {
            record(|span| {
                set_team_attributes(span, team.team_id.as_deref(), team.team_alias.as_deref());
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn identity(input: &Input) -> HelperResult {
    let payload = &input.payload;
    let params = &input.litellm_params;
    let team_keys = &input.baggage_team_metadata_keys;
    let metadata_keys = &input.baggage_metadata_keys;
    record(|span| write_identity_attributes(span, payload, params, team_keys, metadata_keys))
}

fn service_tier(input: &Input) -> HelperResult {
    let mut records = Vec::with_capacity(input.payloads.len());
    for payload in &input.payloads {
        records.push(record(|span| set_service_tier_attributes(span, payload))?);
    }
    Ok(Value::Array(records))
}

fn cache_and_operation(input: &Input) -> HelperResult {
    let cache = input
        .payloads
        .iter()
        .map(|payload| record(|span| set_cache_token_attributes(span, payload)))
        .collect::<Result<Vec<_>, _>>()?;
    let operations: Vec<&str> = input
        .call_types
        .iter()
        .map(|call_type| gen_ai_operation_name(call_type.as_deref()))
        .collect();
    Ok(json!({"cache": cache, "operations": operations}))
}
