use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_exporters::headers::HeadersInput;
use betterlite_logging::otel::legacy_exporters::tracer_cache::CacheKey;
use betterlite_logging::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct ConfigInput {
    exporter: Option<String>,
    endpoint: Option<String>,
    headers: Option<String>,
    service_name: Option<String>,
    deployment_environment: Option<String>,
    model_id: Option<String>,
}

impl ConfigInput {
    pub(super) fn to_config(&self, environment: &BTreeMap<String, String>) -> OpenTelemetryConfig {
        let mut config = self.raw_config();
        config.normalize(environment);
        config
    }

    fn raw_config(&self) -> OpenTelemetryConfig {
        OpenTelemetryConfig {
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
            semconv_stability_opt_in: std::collections::BTreeSet::new(),
            baggage_team_metadata_keys: Vec::new(),
            baggage_metadata_keys: Vec::new(),
            attributes: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum HeadersValue {
    Text(String),
    Map(BTreeMap<String, String>),
}

impl HeadersValue {
    pub(super) fn as_input(&self) -> HeadersInput<'_> {
        match self {
            Self::Text(text) => HeadersInput::Text(text),
            Self::Map(map) => HeadersInput::Map(map),
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct RequestInput {
    pub(super) key: Value,
    pub(super) owns_exporter: bool,
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

pub(super) fn cache_key(value: &Value) -> Option<CacheKey> {
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
