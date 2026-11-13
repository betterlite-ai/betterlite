use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::ExposeSecret as _;

use crate::otel::config::{ExporterOwner, ExporterSpec, Settings};
use crate::otel::integrations::phoenix_settings::PhoenixConfig;
use crate::otel::presets::base::{Preset, PresetError};
use crate::otel::presets::utils::ensure_mappers;

pub struct PhoenixPreset;

impl Preset for PhoenixPreset {
    fn build(
        &self,
        environment: &dyn EnvironmentLookup,
        base: Settings,
        _allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError> {
        let config = PhoenixConfig::from_env(environment)?;
        let project_name = preset_project_name(environment);
        let mut settings = base;
        settings.exporters.push(ExporterSpec {
            kind: config.protocol.exporter_kind().to_owned(),
            endpoint: Some(config.endpoint),
            traces_endpoint: None,
            headers: config
                .otlp_auth_headers
                .map(|header| header.expose_secret().to_owned()),
            owner: Some(ExporterOwner::ArizePhoenix),
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        });
        settings.mapper_names = ensure_mappers(&settings.mapper_names, &["openinference"]);
        settings
            .resource_attributes
            .insert("openinference.project.name".to_owned(), project_name);
        Ok(settings)
    }
}

// LiteLLM's preset reads its own settings class, which also accepts the collector alias
fn preset_project_name(environment: &dyn EnvironmentLookup) -> String {
    ["PHOENIX_PROJECT_NAME", "PHOENIX_COLLECTOR_PROJECT_NAME"]
        .iter()
        .find_map(|name| environment.lookup(name).filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "default".to_owned())
}

#[must_use]
pub fn phoenix_project_headers(
    auth_metadata: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    if let Some(project) = ["phoenix_project_name_override", "phoenix_project_name"]
        .iter()
        .find_map(|key| {
            auth_metadata
                .get(*key)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
        })
    {
        headers.insert("x-project-name".to_owned(), project.to_owned());
    }
    headers
}

mod tests;
