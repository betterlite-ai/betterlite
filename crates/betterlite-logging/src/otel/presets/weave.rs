use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::ExposeSecret as _;

use crate::otel::config::{ExporterOwner, ExporterSpec, Settings};
use crate::otel::integrations::weave_settings::{WeaveOtelConfig, weave_authorization_header};
use crate::otel::presets::base::{Preset, PresetError};
use crate::otel::presets::utils::{credential_gated_exporters, ensure_mappers};

pub struct WeavePreset;

impl Preset for WeavePreset {
    fn build(
        &self,
        environment: &dyn EnvironmentLookup,
        base: Settings,
        allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError> {
        let mappers = ensure_mappers(&base.mapper_names, &["openinference", "weave"]);
        match WeaveOtelConfig::from_env(environment) {
            Ok(config) => {
                let mut settings = base;
                settings.exporters.push(ExporterSpec {
                    kind: config.protocol.exporter_kind().to_owned(),
                    endpoint: Some(config.endpoint),
                    traces_endpoint: None,
                    headers: Some(config.otlp_auth_headers.expose_secret().to_owned()),
                    owner: Some(ExporterOwner::WeaveOtel),
                    options: None,
                    use_simple_processor: None,
                    requires_headers: false,
                });
                settings.mapper_names = mappers;
                Ok(settings)
            }
            Err(_) if allow_missing_credentials => {
                let mut settings = base;
                settings.exporters =
                    credential_gated_exporters(settings.exporters, ExporterOwner::WeaveOtel);
                settings.mapper_names = mappers;
                Ok(settings)
            }
            Err(error) => Err(PresetError::Weave(error)),
        }
    }
}

#[must_use]
pub fn weave_dynamic_headers(params: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    if let Some(api_key) = params
        .get("wandb_api_key")
        .filter(|value| !value.is_empty())
    {
        headers.insert(
            "Authorization".to_owned(),
            weave_authorization_header(api_key),
        );
    }
    if let Some(project_id) = params
        .get("weave_project_id")
        .filter(|value| !value.is_empty())
    {
        headers.insert("project_id".to_owned(), project_id.clone());
    }
    headers
}

mod tests;
