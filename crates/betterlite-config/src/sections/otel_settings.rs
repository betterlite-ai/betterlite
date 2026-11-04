use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Raw {
    callback_settings: Option<CallbackSettings>,
    exporter: Option<String>,
    endpoint: Option<String>,
    traces_endpoint: Option<String>,
    headers: Option<String>,
    service_name: Option<String>,
    deployment_environment: Option<String>,
    enable_metrics: Option<bool>,
    enable_events: Option<bool>,
    capture_message_content: Option<String>,
    legacy_compat: Option<bool>,
    langfuse_span_scope: Option<String>,
    excluded_services: Option<serde_json::Value>,
    exporters: Option<Vec<ExporterSpec>>,
    mapper_names: Option<ListSetting>,
    resource_attributes: Option<std::collections::BTreeMap<String, String>>,
    baggage_promoted_keys: Option<ListSetting>,
    baggage_metadata_keys: Option<ListSetting>,
    baggage_team_metadata_keys: Option<ListSetting>,
}

#[derive(Debug, Default, Deserialize)]
struct CallbackSettings {
    otel: Option<RawOtel>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawOtel {
    exporter: Option<String>,
    endpoint: Option<String>,
    traces_endpoint: Option<String>,
    headers: Option<String>,
    service_name: Option<String>,
    deployment_environment: Option<String>,
    enable_metrics: Option<bool>,
    enable_events: Option<bool>,
    capture_message_content: Option<String>,
    legacy_compat: Option<bool>,
    langfuse_span_scope: Option<String>,
    excluded_services: Option<serde_json::Value>,
    exporters: Option<Vec<ExporterSpec>>,
    mapper_names: Option<ListSetting>,
    resource_attributes: Option<std::collections::BTreeMap<String, String>>,
    baggage_promoted_keys: Option<ListSetting>,
    baggage_metadata_keys: Option<ListSetting>,
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
    pub options: Option<std::collections::BTreeMap<String, String>>,
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
    pub resource_attributes: std::collections::BTreeMap<String, String>,
    pub baggage_promoted_keys: Vec<String>,
    pub baggage_metadata_keys: Vec<String>,
    pub baggage_team_metadata_keys: Vec<String>,
}

mod resolve;

#[path = "otel_settings/tests.rs"]
mod tests;
