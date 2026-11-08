use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_policy::annotations::{
    set_service_tier_attributes, set_team_attributes, write_identity_attributes,
    write_team_attributes,
};
use betterlite_logging::otel::legacy_policy::capture::{CapturePolicy, LoggingFlags};
use betterlite_logging::otel::legacy_policy::config::{
    MetadataKeys, OpenTelemetryConfig, normalize_metadata_keys,
};
use betterlite_logging::otel::legacy_policy::dedupe::EmittedSpans;
use betterlite_logging::otel::legacy_policy::error_recording::{
    RecordedException, record_exception_on_span,
};
use betterlite_logging::otel::legacy_policy::semconv_attrs::{
    gen_ai_operation_name, set_cache_token_attributes, set_semconv_request_attributes,
};
use betterlite_logging::otel::legacy_policy::span_time::{SpanTime, span_name, to_ns};
use serde_json::{Value, json};

use super::config_input::ConfigInput;
use super::inputs::{
    EmitCall, IdentityInput, MetadataInput, ModelMap, SpanNameInput, TeamInput, TimeInput,
};
use super::render::{config_json, record};
use crate::helpers::HelperResult;

pub(super) fn config_from_env(environment: &BTreeMap<String, String>) -> Value {
    let mut config = OpenTelemetryConfig::from_env(environment);
    config.normalize(environment);
    config_json(&config)
}

pub(super) fn config_normalize(
    input: &ConfigInput,
    environment: &BTreeMap<String, String>,
) -> Value {
    let mut config = input.to_config();
    config.normalize(environment);
    config_json(&config)
}

pub(super) fn metadata_keys(values: &[MetadataInput]) -> Value {
    let normalized: Vec<Vec<String>> = values
        .iter()
        .map(|value| match value {
            MetadataInput::Text(text) => normalize_metadata_keys(MetadataKeys::Text(text)),
            MetadataInput::List(items) => normalize_metadata_keys(MetadataKeys::List(items)),
        })
        .collect();
    json!(normalized)
}

pub(super) fn capture_sample(
    configs: &[ConfigInput],
    environment: &BTreeMap<String, String>,
) -> Value {
    let sampled: Vec<Value> = configs
        .iter()
        .map(|config| {
            let policy = CapturePolicy::from_init_state(&config.to_config(), environment);
            let without = policy.resolve(LoggingFlags {
                turn_off_message_logging: false,
                message_logging: false,
            });
            let with = policy.resolve(LoggingFlags {
                turn_off_message_logging: false,
                message_logging: true,
            });
            json!({
                "without_message_logging": without.as_str(),
                "with_message_logging": with.as_str(),
            })
        })
        .collect();
    Value::Array(sampled)
}

pub(super) fn capture_resolve(config: &ConfigInput, flags: LoggingFlags) -> Value {
    let policy = CapturePolicy::from_init_state(&config.to_config(), &BTreeMap::new());
    let mode = policy.resolve(flags);
    json!({
        "mode": mode.as_str(),
        "in_span": mode.capture_in_span(),
        "in_event": mode.capture_in_event(),
    })
}

pub(super) fn emit_once(calls: &[EmitCall]) -> Value {
    let mut spans = EmittedSpans::default();
    let emitted: Vec<bool> = calls
        .iter()
        .map(|call| spans.emit_once(&call.handler, &call.scope))
        .collect();
    json!(emitted)
}

pub(super) fn span_time(times: &[TimeInput], now: i64, names: &[SpanNameInput]) -> Value {
    let nanoseconds: Vec<i64> = times
        .iter()
        .map(|time| to_ns(span_time_value(time), now))
        .collect();
    let resolved: Vec<String> = names
        .iter()
        .map(|name| {
            span_name(
                &name.litellm_params,
                gen_ai_operation_name(name.call_type.as_deref()),
                name.model.as_deref(),
                name.semconv,
            )
        })
        .collect();
    json!({"ns": nanoseconds, "names": resolved})
}

pub(super) fn team_attributes(teams: &[TeamInput]) -> HelperResult {
    let mut records = Vec::with_capacity(teams.len());
    for team in teams {
        records.push(record(|span| {
            set_team_attributes(span, team.team_id.as_deref(), team.team_alias.as_deref());
        })?);
    }
    Ok(Value::Array(records))
}

pub(super) fn team_from_payload(payload: &ModelMap) -> HelperResult {
    record(|span| {
        write_team_attributes(span, payload);
    })
}

pub(super) fn identity(input: &IdentityInput) -> HelperResult {
    record(|span| {
        write_identity_attributes(
            span,
            &input.payload,
            &input.litellm_params,
            &input.baggage_team_metadata_keys,
            &input.baggage_metadata_keys,
        );
    })
}

pub(super) fn service_tier(payloads: &[ModelMap]) -> HelperResult {
    let mut records = Vec::with_capacity(payloads.len());
    for payload in payloads {
        records.push(record(|span| {
            set_service_tier_attributes(span, payload);
        })?);
    }
    Ok(Value::Array(records))
}

pub(super) fn exception_stamp(
    exception: Option<&RecordedException>,
    payload: &ModelMap,
) -> HelperResult {
    record(|span| {
        record_exception_on_span(span, exception, payload);
    })
}

pub(super) fn semconv_request(optional_params: &ModelMap) -> HelperResult {
    record(|span| {
        set_semconv_request_attributes(span, optional_params);
    })
}

pub(super) fn semconv_cache_and_operation(
    payloads: &[ModelMap],
    call_types: &[Option<String>],
) -> HelperResult {
    let mut cache = Vec::with_capacity(payloads.len());
    for payload in payloads {
        cache.push(record(|span| {
            set_cache_token_attributes(span, payload);
        })?);
    }
    let operations: Vec<&str> = call_types
        .iter()
        .map(|call_type| gen_ai_operation_name(call_type.as_deref()))
        .collect();
    Ok(json!({"cache": cache, "operations": operations}))
}

const fn span_time_value(input: &TimeInput) -> SpanTime {
    match input {
        TimeInput::Seconds(seconds) => SpanTime::Seconds(*seconds),
        TimeInput::DateTime(datetime) => SpanTime::DateTime(*datetime),
    }
}
