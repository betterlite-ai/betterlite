use std::collections::{BTreeMap, BTreeSet};

use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::semconv::{
    OTELSemconvCategory, STABILITY_OPT_IN_ENV, parse_semconv_opt_in,
};

mod tests;

pub const CONSOLE_EXPORTER: &str = "console";
pub const IN_MEMORY_EXPORTER: &str = "in_memory";
pub const OTLP_HTTP_EXPORTER: &str = "otlp_http";
pub const DEFAULT_SERVICE_NAME: &str = "litellm";
pub const DEFAULT_ENVIRONMENT: &str = "production";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Exporter {
    #[default]
    Console,
    InMemory,
    Named(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
pub struct Signals {
    pub enable_metrics: bool,
    pub enable_events: bool,
}

#[derive(Debug, Clone, Default)]
pub struct OpenTelemetryConfig {
    pub exporter: Exporter,
    pub endpoint: Option<String>,
    pub headers: Option<String>,
    pub signals: Signals,
    pub service_name: String,
    pub deployment_environment: String,
    pub model_id: String,
    pub ignore_context_propagation: bool,
    pub skip_set_global: bool,
    pub capture_message_content: Option<String>,
    pub semconv_stability_opt_in: BTreeSet<OTELSemconvCategory>,
    pub baggage_team_metadata_keys: Vec<String>,
    pub baggage_metadata_keys: Vec<String>,
    pub attributes: Option<OtelModelValue>,
}

#[derive(Debug, Clone, Copy)]
pub enum MetadataKeys<'source> {
    Text(&'source str),
    List(&'source [String]),
}

#[must_use]
pub fn normalize_metadata_keys(keys: MetadataKeys<'_>) -> Vec<String> {
    let parts: Vec<&str> = match keys {
        MetadataKeys::Text(text) => text.split(',').collect(),
        MetadataKeys::List(items) => items.iter().map(String::as_str).collect(),
    };
    parts
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

const EMPTY: OpenTelemetryConfig = OpenTelemetryConfig {
    exporter: Exporter::Console,
    endpoint: None,
    headers: None,
    signals: Signals {
        enable_metrics: false,
        enable_events: false,
    },
    service_name: String::new(),
    deployment_environment: String::new(),
    model_id: String::new(),
    ignore_context_propagation: false,
    skip_set_global: false,
    capture_message_content: None,
    semconv_stability_opt_in: BTreeSet::new(),
    baggage_team_metadata_keys: Vec::new(),
    baggage_metadata_keys: Vec::new(),
    attributes: None,
};

impl OpenTelemetryConfig {
    #[must_use]
    pub fn from_env(env: &BTreeMap<String, String>) -> Self {
        let exporter = first_present(env, &["OTEL_EXPORTER_OTLP_PROTOCOL", "OTEL_EXPORTER"])
            .unwrap_or_else(|| CONSOLE_EXPORTER.to_owned());
        if exporter == IN_MEMORY_EXPORTER {
            let mut config = EMPTY;
            config.exporter = Exporter::InMemory;
            return config;
        }
        let mut config = EMPTY;
        config.exporter = match exporter.as_str() {
            CONSOLE_EXPORTER => Exporter::Console,
            IN_MEMORY_EXPORTER => Exporter::InMemory,
            _ => Exporter::Named(exporter.clone()),
        };
        config.endpoint = first_present(env, &["OTEL_EXPORTER_OTLP_ENDPOINT", "OTEL_ENDPOINT"]);
        config.headers = first_present(env, &["OTEL_EXPORTER_OTLP_HEADERS", "OTEL_HEADERS"]);
        config.service_name = first_present(env, &["OTEL_SERVICE_NAME"])
            .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_owned());
        config.signals = Signals {
            enable_metrics: enabled(env, "LITELLM_OTEL_INTEGRATION_ENABLE_METRICS"),
            enable_events: enabled(env, "LITELLM_OTEL_INTEGRATION_ENABLE_EVENTS"),
        };
        config.model_id =
            first_present(env, &["OTEL_MODEL_ID"]).unwrap_or_else(|| config.service_name.clone());
        config.deployment_environment = first_present(env, &["OTEL_ENVIRONMENT_NAME"])
            .unwrap_or_else(|| DEFAULT_ENVIRONMENT.to_owned());
        config
    }

    pub fn normalize(&mut self, env: &BTreeMap<String, String>) {
        if self.endpoint.is_some() && self.exporter == Exporter::Console {
            self.exporter = Exporter::Named(OTLP_HTTP_EXPORTER.to_owned());
        }
        if self.service_name.is_empty() {
            self.service_name = first_present(env, &["OTEL_SERVICE_NAME"])
                .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_owned());
        }
        if self.deployment_environment.is_empty() {
            self.deployment_environment = first_present(env, &["OTEL_ENVIRONMENT_NAME"])
                .unwrap_or_else(|| DEFAULT_ENVIRONMENT.to_owned());
        }
        if self.model_id.is_empty() {
            self.model_id =
                first_present(env, &["OTEL_MODEL_ID"]).unwrap_or_else(|| self.service_name.clone());
        }
        if !self.ignore_context_propagation {
            self.ignore_context_propagation = env
                .get("OTEL_IGNORE_CONTEXT_PROPAGATION")
                .is_some_and(|value| value.trim().to_lowercase() == "true");
        }
        let opt_in = env.get(STABILITY_OPT_IN_ENV).map_or("", String::as_str);
        self.semconv_stability_opt_in
            .extend(parse_semconv_opt_in(opt_in));
        normalize_baggage_keys(
            &mut self.baggage_team_metadata_keys,
            env.get("LITELLM_OTEL_BAGGAGE_TEAM_METADATA_KEYS"),
        );
        normalize_baggage_keys(
            &mut self.baggage_metadata_keys,
            env.get("LITELLM_OTEL_BAGGAGE_METADATA_KEYS"),
        );
    }
}

fn normalize_baggage_keys(keys: &mut Vec<String>, from_env: Option<&String>) {
    if !keys.is_empty() {
        let normalized = normalize_metadata_keys(MetadataKeys::List(keys));
        *keys = normalized;
    }
    if keys.is_empty() {
        let text = from_env.map_or("", String::as_str);
        *keys = normalize_metadata_keys(MetadataKeys::Text(text));
    }
}

fn first_present(env: &BTreeMap<String, String>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| env.get(*name).filter(|value| !value.is_empty()).cloned())
}

fn enabled(env: &BTreeMap<String, String>, name: &str) -> bool {
    env.get(name)
        .is_some_and(|value| value.to_lowercase() == "true")
}
