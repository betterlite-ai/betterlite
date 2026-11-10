use std::collections::{BTreeMap, BTreeSet};

use betterlite_logging::otel::config::Signal;
use betterlite_logging::otel::legacy_exporters::endpoint::normalize_otel_endpoint;
use betterlite_logging::otel::legacy_exporters::headers::{HeadersInput, headers_dictionary};
use betterlite_logging::otel::legacy_exporters::resource::legacy_resource;
use betterlite_logging::otel::legacy_exporters::selection::{
    log_exporter_plan, metric_reader_plan, span_processor_plan,
};
use betterlite_logging::otel::legacy_exporters::tracer_cache::CacheKey;
use betterlite_logging::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::HelperResult;

use self::support::{invalid, plan_json, resource_json, span_plan_json, tracer_cache_sequence};

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

#[derive(Deserialize, Default)]
#[serde(default)]
struct ConfigInput {
    exporter: Option<String>,
    endpoint: Option<String>,
    headers: Option<String>,
    service_name: Option<String>,
    deployment_environment: Option<String>,
    model_id: Option<String>,
}

impl ConfigInput {
    fn to_config(&self, environment: &BTreeMap<String, String>) -> OpenTelemetryConfig {
        let mut config = OpenTelemetryConfig {
            exporter: match self.exporter.as_deref() {
                Some("in_memory") => Exporter::InMemory,
                Some("console") | None => Exporter::Console,
                Some(text) => Exporter::Named(text.to_owned()),
            },
            endpoint: self.endpoint.clone(),
            headers: self.headers.clone(),
            signals: Signals::default(),
            service_name: self.service_name.clone().unwrap_or_default(),
            deployment_environment: self.deployment_environment.clone().unwrap_or_default(),
            model_id: self.model_id.clone().unwrap_or_default(),
            ignore_context_propagation: false,
            skip_set_global: false,
            capture_message_content: None,
            semconv_stability_opt_in: BTreeSet::new(),
            baggage_team_metadata_keys: Vec::new(),
            baggage_metadata_keys: Vec::new(),
            attributes: None,
        };
        config.normalize(environment);
        config
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HeadersValue {
    Text(String),
    Map(BTreeMap<String, String>),
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RequestInput {
    key: Value,
    owns_exporter: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum KeyInput {
    Config { config: CacheConfigInput },
    Headers { headers: BTreeMap<String, String> },
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CacheConfigInput {
    exporter: Option<String>,
    endpoint: Option<String>,
    headers: Option<String>,
}

fn cache_key(value: &Value) -> Option<CacheKey> {
    let key = serde_json::from_value::<KeyInput>(value.clone()).ok()?;
    Some(match key {
        KeyInput::Config { config } => CacheKey::Config {
            exporter: config.exporter.unwrap_or_else(|| "console".to_owned()),
            endpoint: config.endpoint,
            headers: config.headers,
        },
        KeyInput::Headers { headers } => CacheKey::Headers(headers),
    })
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
    let env = &input.environment;
    let base = input.config.to_config(env);
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
    Ok(span_plan_json(&plan))
}

fn log_exporter_reply(input: &Input) -> HelperResult {
    let config = input.config.to_config(&input.environment);
    let plan = log_exporter_plan(&config, input.callback_name.as_deref(), &input.environment)
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(plan_json(&plan))
}

fn metric_reader_reply(input: &Input) -> HelperResult {
    let config = input.config.to_config(&input.environment);
    let plan = metric_reader_plan(&config, input.callback_name.as_deref())
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(plan_json(&plan))
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
