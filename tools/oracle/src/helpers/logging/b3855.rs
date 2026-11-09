use std::collections::BTreeMap;

use betterlite_logging::otel::config::Signal;
use betterlite_logging::otel::legacy_exporters::endpoint::normalize_otel_endpoint;
use betterlite_logging::otel::legacy_exporters::headers::{HeadersInput, headers_dictionary};
use betterlite_logging::otel::legacy_exporters::resource::legacy_resource;
use betterlite_logging::otel::legacy_exporters::selection::{
    log_exporter_plan, metric_reader_plan, span_processor_plan,
};
use serde::Deserialize;

use crate::helpers::HelperResult;

use self::inputs::{ConfigInput, HeadersValue, RequestInput};
use self::support::{invalid, plan_fields, resource_json, span_plan_json, tracer_cache_sequence};

mod inputs;
mod support;
mod tests;

pub(crate) struct Op;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Input {
    function: String,
    endpoint: Option<String>,
    signal: Option<String>,
    callback_name: Option<String>,
    headers: Option<HeadersValue>,
    config: ConfigInput,
    config_override: Option<ConfigInput>,
    dynamic_headers: Option<BTreeMap<String, String>>,
    environment: BTreeMap<String, String>,
    max_providers: Option<usize>,
    requests: Vec<RequestInput>,
}

fn parse_signal(text: &str) -> Option<Signal> {
    match text {
        "traces" => Some(Signal::Traces),
        "metrics" => Some(Signal::Metrics),
        "logs" => Some(Signal::Logs),
        _ => None,
    }
}

fn headers_dictionary_reply(input: &Input) -> HelperResult {
    let headers = input
        .headers
        .as_ref()
        .map_or_else(
            || headers_dictionary(HeadersInput::None),
            |value| headers_dictionary(value.as_input()),
        )
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(serde_json::json!(headers))
}

fn span_processor_reply(input: &Input) -> HelperResult {
    let base = input.config.to_config(&input.environment);
    let config_override = input
        .config_override
        .as_ref()
        .map(|config| config.to_config(&input.environment));
    let plan = span_processor_plan(
        &base,
        input.dynamic_headers.as_ref(),
        config_override.as_ref(),
        input.callback_name.as_deref(),
    )
    .map_err(|error| invalid(&error.to_string()))?;
    Ok(span_plan_json(&plan))
}

fn log_exporter_reply(input: &Input) -> HelperResult {
    let plan = log_exporter_plan(
        &input.config.to_config(&input.environment),
        input.callback_name.as_deref(),
        &input.environment,
    )
    .map_err(|error| invalid(&error.to_string()))?;
    Ok(plan_fields(
        plan.kind,
        plan.endpoint.as_deref(),
        &plan.headers,
    ))
}

fn metric_reader_reply(input: &Input) -> HelperResult {
    let plan = metric_reader_plan(
        &input.config.to_config(&input.environment),
        input.callback_name.as_deref(),
    )
    .map_err(|error| invalid(&error.to_string()))?;
    Ok(plan_fields(
        plan.kind,
        plan.endpoint.as_deref(),
        &plan.headers,
    ))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function.as_str() {
            "normalize_otel_endpoint" => {
                let Some(signal) = input.signal.as_deref().and_then(parse_signal) else {
                    return Err(invalid("the signal must be traces, metrics or logs"));
                };
                let normalized = normalize_otel_endpoint(
                    input.endpoint.as_deref(),
                    signal,
                    input.callback_name.as_deref(),
                );
                Ok(serde_json::json!(normalized))
            }
            "headers_dictionary" => headers_dictionary_reply(input),
            "span_processor_plan" => span_processor_reply(input),
            "log_exporter_plan" => log_exporter_reply(input),
            "metric_reader_plan" => metric_reader_reply(input),
            "tracer_cache_sequence" => tracer_cache_sequence(input),
            "litellm_resource" => Ok(resource_json(&legacy_resource(
                &input.config.to_config(&input.environment),
                &input.environment,
            ))),
            other => Err(invalid(&format!("unknown function {other}"))),
        }
    }
}
