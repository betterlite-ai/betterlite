#![expect(
    clippy::struct_excessive_bools,
    reason = "OTel exposes independent feature switches"
)]

use crate::loader::{ConfigError, EnvironmentLookup};
use serde::{Deserialize, de::IgnoredAny};
use std::collections::BTreeMap;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    callback_settings: Option<CallbackSettings>,
}

#[derive(Debug, Default, Deserialize)]
struct CallbackSettings {
    otel: Option<RawOtel>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawOtel {
    #[serde(alias = "OTEL_EXPORTER", alias = "OTEL_EXPORTER_OTLP_PROTOCOL")]
    exporter: Option<String>,
    #[serde(alias = "OTEL_ENDPOINT", alias = "OTEL_EXPORTER_OTLP_ENDPOINT")]
    endpoint: Option<String>,
    #[serde(
        alias = "OTEL_TRACES_ENDPOINT",
        alias = "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"
    )]
    traces_endpoint: Option<String>,
    #[serde(alias = "OTEL_HEADERS", alias = "OTEL_EXPORTER_OTLP_HEADERS")]
    headers: Option<String>,
    #[serde(alias = "OTEL_SERVICE_NAME")]
    service_name: Option<String>,
    #[serde(alias = "OTEL_ENVIRONMENT_NAME")]
    deployment_environment: Option<String>,
    #[serde(alias = "LITELLM_OTEL_INTEGRATION_ENABLE_METRICS")]
    enable_metrics: Option<bool>,
    #[serde(alias = "LITELLM_OTEL_INTEGRATION_ENABLE_EVENTS")]
    enable_events: Option<bool>,
    #[serde(alias = "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT")]
    capture_message_content: Option<String>,
    #[serde(alias = "LITELLM_OTEL_LEGACY_COMPAT")]
    legacy_compat: Option<bool>,
    #[serde(alias = "LITELLM_OTEL_LANGFUSE_SPAN_SCOPE")]
    langfuse_span_scope: Option<String>,
    #[serde(alias = "LITELLM_OTEL_EXCLUDED_SERVICES")]
    excluded_services: Option<ExclusionInput>,
    exporters: Option<Vec<ExporterSpec>>,
    mapper_names: Option<ListSetting>,
    resource_attributes: Option<BTreeMap<String, String>>,
    #[serde(alias = "LITELLM_OTEL_BAGGAGE_PROMOTED_KEYS")]
    baggage_promoted_keys: Option<ListSetting>,
    #[serde(alias = "LITELLM_OTEL_BAGGAGE_METADATA_KEYS")]
    baggage_metadata_keys: Option<ListSetting>,
    #[serde(alias = "LITELLM_OTEL_BAGGAGE_TEAM_METADATA_KEYS")]
    baggage_team_metadata_keys: Option<ListSetting>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ListSetting {
    List(Vec<String>),
    Csv(String),
}

impl ListSetting {
    fn values(self) -> Vec<String> {
        match self {
            Self::List(values) => values,
            Self::Csv(value) => resolve::csv_values(&value),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExporterSpec {
    pub kind: Option<String>,
    pub endpoint: Option<String>,
    pub traces_endpoint: Option<String>,
    pub headers: Option<String>,
    pub owner: Option<String>,
    pub options: Option<BTreeMap<String, String>>,
    pub use_simple_processor: Option<bool>,
    pub requires_headers: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub exporter: String,
    pub endpoint: Option<String>,
    pub traces_endpoint: Option<String>,
    pub headers: Option<String>,
    pub service_name: String,
    pub deployment_environment: Option<String>,
    pub enable_metrics: bool,
    pub enable_events: bool,
    pub capture_message_content: String,
    pub legacy_compat: bool,
    pub langfuse_span_scope: String,
    pub excluded_services: Vec<String>,
    pub exporters: Vec<ExporterSpec>,
    pub mapper_names: Vec<String>,
    pub resource_attributes: BTreeMap<String, String>,
    pub baggage_promoted_keys: Vec<String>,
    pub baggage_metadata_keys: Vec<String>,
    pub baggage_team_metadata_keys: Vec<String>,
}

mod resolve;

#[path = "otel_settings/tests.rs"]
mod tests;

#[derive(Default)]
pub struct FeatureFlag {
    enabled: Option<bool>,
}

impl FeatureFlag {
    pub fn enabled<L>(&mut self, lookup: &L) -> Result<bool, ConfigError>
    where
        L: EnvironmentLookup,
    {
        if let Some(enabled) = self.enabled {
            return Ok(enabled);
        }
        let value = lookup.lookup("LITELLM_OTEL_V2").unwrap_or_default();
        let enabled = betterlite_env::flag("LITELLM_OTEL_V2", &value)
            .map_err(|source| ConfigError::Environment { source })?
            .unwrap_or(false);
        self.enabled = Some(enabled);
        Ok(enabled)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ExclusionInput {
    Csv(String),
    List(Vec<Self>),
    Ignored(IgnoredAny),
}
