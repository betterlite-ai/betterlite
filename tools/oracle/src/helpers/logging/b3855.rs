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
use self::support::{invalid, plan_json, resource_json, tracer_cache_sequence};

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

fn headers_dictionary_reply(input: &Input) -> HelperResult {
    let source = match input.headers.as_ref() {
        Some(HeadersValue::Text(text)) => HeadersInput::Text(text),
        Some(HeadersValue::Map(map)) => HeadersInput::Map(map),
        None => HeadersInput::None,
    };
    let headers = headers_dictionary(source).map_err(|error| invalid(&error.to_string()))?;
    Ok(serde_json::json!(headers))
}

fn span_processor_reply(input: &Input) -> HelperResult {
    let base = input.config.to_config(&input.environment);
    let env = &input.environment;
    let config_override = input
        .config_override
        .as_ref()
        .map(|config| config.to_config(env));
    let plan = span_processor_plan(
        &base,
        input.dynamic_headers.as_ref(),
        config_override.as_ref(),
        input.callback_name.as_deref(),
    )
    .map_err(|error| invalid(&error.to_string()))?;
    Ok(plan_json(
        plan.kind,
        plan.endpoint.as_deref(),
        &plan.headers,
        Some(plan.processor),
    ))
}

fn log_exporter_reply(input: &Input) -> HelperResult {
    let config = input.config.to_config(&input.environment);
    let plan = log_exporter_plan(&config, input.callback_name.as_deref(), &input.environment)
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(plan_json(
        plan.kind,
        plan.endpoint.as_deref(),
        &plan.headers,
        None,
    ))
}

fn metric_reader_reply(input: &Input) -> HelperResult {
    let config = input.config.to_config(&input.environment);
    let plan = metric_reader_plan(&config, input.callback_name.as_deref())
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(plan_json(
        plan.kind,
        plan.endpoint.as_deref(),
        &plan.headers,
        None,
    ))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function.as_str() {
            "normalize_otel_endpoint" => {
                let signal = match input.signal.as_deref() {
                    Some("traces") => Signal::Traces,
                    Some("metrics") => Signal::Metrics,
                    Some("logs") => Signal::Logs,
                    _ => return Err(invalid("the signal must be traces, metrics or logs")),
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
