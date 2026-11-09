use std::collections::BTreeMap;

use betterlite_logging::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use betterlite_logging::otel::legacy_policy::dedupe::{HandlerIdentity, ScopeValue};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::vocabulary::semconv::OTELSemconvCategory;
use chrono::{DateTime, Utc};
use serde::Deserialize;

pub(super) type ModelMap = BTreeMap<String, OtelModelValue>;

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct ConfigInput {
    exporter: Option<String>,
    endpoint: Option<String>,
    headers: Option<String>,
    signals: Signals,
    service_name: Option<String>,
    deployment_environment: Option<String>,
    model_id: Option<String>,
    ignore_context_propagation: bool,
    skip_set_global: bool,
    capture_message_content: Option<String>,
    semconv_stability_opt_in: Vec<String>,
    baggage_team_metadata_keys: Vec<String>,
    baggage_metadata_keys: Vec<String>,
}

impl ConfigInput {
    pub(super) fn to_config(&self) -> OpenTelemetryConfig {
        OpenTelemetryConfig {
            exporter: match self.exporter.as_deref() {
                None | Some("console") => Exporter::Console,
                Some("in_memory") => Exporter::InMemory,
                Some(item) => Exporter::Named(item.to_owned()),
            },
            endpoint: self.endpoint.clone(),
            headers: self.headers.clone(),
            signals: self.signals,
            service_name: self.service_name.clone().unwrap_or_default(),
            deployment_environment: self.deployment_environment.clone().unwrap_or_default(),
            model_id: self.model_id.clone().unwrap_or_default(),
            ignore_context_propagation: self.ignore_context_propagation,
            skip_set_global: self.skip_set_global,
            capture_message_content: self.capture_message_content.clone(),
            semconv_stability_opt_in: self
                .semconv_stability_opt_in
                .iter()
                .filter_map(|item| item.parse::<OTELSemconvCategory>().ok())
                .collect(),
            baggage_team_metadata_keys: self.baggage_team_metadata_keys.clone(),
            baggage_metadata_keys: self.baggage_metadata_keys.clone(),
            attributes: None,
        }
    }
}

#[derive(Deserialize)]
pub(super) struct Call {
    pub(super) handler: HandlerIdentity,
    #[serde(default)]
    pub(super) scope: Vec<ScopeValue>,
}

#[derive(Deserialize)]
pub(super) struct TeamInput {
    pub(super) team_id: Option<String>,
    pub(super) team_alias: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(super) struct NameInput {
    pub(super) litellm_params: ModelMap,
    pub(super) call_type: Option<String>,
    pub(super) model: Option<String>,
    pub(super) semconv: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum TimeInput {
    Seconds(f64),
    DateTime(DateTime<Utc>),
}
