use std::collections::BTreeMap;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::ExposeSecret as _;

use crate::otel::config::{ExporterOwner, ExporterSpec, Settings};
use crate::otel::integrations::arize_settings::ArizeConfig;
use crate::otel::presets::base::{Preset, PresetError};
use crate::otel::presets::utils::ensure_mappers;

pub struct ArizePreset;

impl Preset for ArizePreset {
    fn build(
        &self,
        environment: &dyn EnvironmentLookup,
        base: Settings,
        _allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError> {
        let config = ArizeConfig::from_env(environment);
        let headers = arize_headers(environment, &config);
        let mut settings = base;
        settings.exporters.push(ExporterSpec {
            kind: config.protocol.exporter_kind().to_owned(),
            endpoint: Some(config.endpoint),
            traces_endpoint: None,
            headers,
            owner: Some(ExporterOwner::ArizeAx),
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        });
        settings.mapper_names = ensure_mappers(&settings.mapper_names, &["openinference"]);
        if let Some(project_name) = config.project_name {
            settings
                .resource_attributes
                .insert("model_id".to_owned(), project_name);
        }
        Ok(settings)
    }
}

fn arize_headers(environment: &dyn EnvironmentLookup, config: &ArizeConfig) -> Option<String> {
    let mut pieces = Vec::new();
    if let Some(space_id) = config.space_id.clone().or_else(|| config.space_key.clone()) {
        pieces.push(format!("space_id={space_id}"));
    }
    if let Some(api_key) = &config.api_key {
        pieces.push(format!("api_key={}", api_key.expose_secret()));
    }
    if pieces.is_empty() {
        return environment
            .lookup("OTEL_EXPORTER_OTLP_TRACES_HEADERS")
            .filter(|value| !value.is_empty());
    }
    Some(pieces.join(","))
}

#[must_use]
pub fn arize_dynamic_headers(params: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    let space_key = params
        .get("arize_space_key")
        .filter(|value| !value.is_empty());
    let space_id = params
        .get("arize_space_id")
        .filter(|value| !value.is_empty());
    if let Some(space) = space_key.or(space_id) {
        headers.insert("arize-space-id".to_owned(), space.clone());
    }
    if let Some(api_key) = params
        .get("arize_api_key")
        .filter(|value| !value.is_empty())
    {
        headers.insert("api_key".to_owned(), api_key.clone());
    }
    headers
}

mod tests;
