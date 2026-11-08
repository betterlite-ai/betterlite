use betterlite_logging::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use betterlite_logging::otel::vocabulary::semconv::OTELSemconvCategory;
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub(crate) struct ConfigInput {
    pub(crate) exporter: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) headers: Option<String>,
    #[serde(default)]
    pub(crate) signals: SignalsInput,
    pub(crate) service_name: Option<String>,
    pub(crate) deployment_environment: Option<String>,
    pub(crate) model_id: Option<String>,
    #[serde(default)]
    pub(crate) ignore_context_propagation: bool,
    #[serde(default)]
    pub(crate) skip_set_global: bool,
    pub(crate) capture_message_content: Option<String>,
    #[serde(default)]
    pub(crate) semconv_stability_opt_in: Vec<String>,
    #[serde(default)]
    pub(crate) baggage_team_metadata_keys: Vec<String>,
    #[serde(default)]
    pub(crate) baggage_metadata_keys: Vec<String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct SignalsInput {
    #[serde(default)]
    pub(crate) enable_metrics: bool,
    #[serde(default)]
    pub(crate) enable_events: bool,
}

impl ConfigInput {
    pub(crate) fn to_config(&self) -> OpenTelemetryConfig {
        OpenTelemetryConfig {
            exporter: self.exporter(),
            endpoint: self.endpoint.clone(),
            headers: self.headers.clone(),
            signals: Signals {
                enable_metrics: self.signals.enable_metrics,
                enable_events: self.signals.enable_events,
            },
            service_name: self.service_name.clone().unwrap_or_default(),
            deployment_environment: self.deployment_environment.clone().unwrap_or_default(),
            model_id: self.model_id.clone().unwrap_or_default(),
            ignore_context_propagation: self.ignore_context_propagation,
            skip_set_global: self.skip_set_global,
            capture_message_content: self.capture_message_content.clone(),
            semconv_stability_opt_in: self
                .semconv_stability_opt_in
                .iter()
                .filter_map(|text| text.parse::<OTELSemconvCategory>().ok())
                .collect(),
            baggage_team_metadata_keys: self.baggage_team_metadata_keys.clone(),
            baggage_metadata_keys: self.baggage_metadata_keys.clone(),
            attributes: None,
        }
    }

    fn exporter(&self) -> Exporter {
        match self.exporter.as_deref() {
            None | Some("console") => Exporter::Console,
            Some("in_memory") => Exporter::InMemory,
            Some(text) => Exporter::Named(text.to_owned()),
        }
    }
}
