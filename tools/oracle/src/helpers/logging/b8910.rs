use std::collections::{BTreeMap, BTreeSet};

use betterlite_logging::otel::config::destination::SpanScope;
use betterlite_logging::otel::config::{CaptureMessageContent, ExporterSpec, Settings};
use betterlite_logging::otel::logger_runtime::factory::logger_class;
use betterlite_logging::otel::logger_runtime::preset::{
    exports_nowhere, has_operator_exporter, is_credential_gated, is_gated, only_the_gated_exporter,
};
use betterlite_logging::otel::logger_runtime::types::LoggerClass;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Function {
    ExportsNowhere,
    HasOperatorExporter,
    IsCredentialGated,
    IsGated,
    LoggerClass,
    OnlyTheGatedExporter,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    config: ConfigInput,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigInput {
    exporters: Vec<ExporterInput>,
    mapper_names: Vec<String>,
    capture_message_content: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExporterInput {
    kind: Option<String>,
    endpoint: Option<String>,
    traces_endpoint: Option<String>,
    headers: Option<String>,
    #[serde(default)]
    requires_headers: bool,
}

impl ConfigInput {
    // what B3842's Settings model holds for these fields, with the defaults the model would fill in
    fn settings(&self) -> Settings {
        Settings {
            exporters: self.exporters.iter().map(ExporterInput::spec).collect(),
            service_name: "litellm".to_owned(),
            deployment_environment: None,
            enable_metrics: false,
            enable_events: false,
            capture_message_content: capture_mode(&self.capture_message_content),
            legacy_compat: false,
            langfuse_span_scope: SpanScope::Full,
            excluded_services: BTreeSet::new(),
            mapper_names: self.mapper_names.clone(),
            resource_attributes: BTreeMap::new(),
            baggage_promoted_keys: Vec::new(),
            baggage_metadata_keys: Vec::new(),
            baggage_team_metadata_keys: Vec::new(),
        }
    }
}

impl ExporterInput {
    fn spec(&self) -> ExporterSpec {
        ExporterSpec {
            kind: self.kind.clone().unwrap_or_else(|| "console".to_owned()),
            endpoint: self.endpoint.clone(),
            traces_endpoint: self.traces_endpoint.clone(),
            headers: self.headers.clone(),
            owner: None,
            options: None,
            use_simple_processor: None,
            requires_headers: self.requires_headers,
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let config = input.config.settings();
        let value = match input.function {
            Function::LoggerClass => json!(class_name(logger_class(&config))),
            Function::IsGated => {
                json!(config.exporters.iter().map(is_gated).collect::<Vec<bool>>())
            }
            Function::ExportsNowhere => json!(exports_nowhere(&config)),
            Function::IsCredentialGated => json!(is_credential_gated(&config)),
            Function::HasOperatorExporter => json!(has_operator_exporter(&config)),
            Function::OnlyTheGatedExporter => json!(
                only_the_gated_exporter(&config)
                    .exporters
                    .iter()
                    .map(exporter_value)
                    .collect::<Vec<Value>>()
            ),
        };
        Ok(value)
    }
}

// the three LiteLLM callback classes _logger_class can return
fn class_name(class: LoggerClass) -> &'static str {
    match class {
        LoggerClass::Generic => "OpenTelemetryV2",
        LoggerClass::Langfuse => "LangfuseOpenTelemetryV2",
        LoggerClass::LangfuseContent => "LangfuseContentOpenTelemetryV2",
    }
}

fn capture_mode(value: &str) -> CaptureMessageContent {
    match value {
        "span_only" => CaptureMessageContent::SpanOnly,
        "event_only" => CaptureMessageContent::EventOnly,
        "span_and_event" => CaptureMessageContent::SpanAndEvent,
        _ => CaptureMessageContent::NoContent,
    }
}

fn exporter_value(spec: &ExporterSpec) -> Value {
    json!({
        "kind": &spec.kind,
        "endpoint": &spec.endpoint,
        "traces_endpoint": &spec.traces_endpoint,
        "headers": &spec.headers,
        "requires_headers": spec.requires_headers,
    })
}
