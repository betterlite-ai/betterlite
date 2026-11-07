use betterlite_config::loader::EnvironmentLookup;

use crate::otel::config::{ExporterOwner, ExporterSpec, Settings};
use crate::otel::integrations::levo_settings::LevoConfig;
use crate::otel::presets::base::{Preset, PresetError};

pub struct LevoPreset;

impl Preset for LevoPreset {
    fn build(
        &self,
        environment: &dyn EnvironmentLookup,
        base: Settings,
        _allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError> {
        let config = LevoConfig::from_env(environment)?;
        let mut settings = base;
        settings.exporters.push(ExporterSpec {
            kind: config.protocol.exporter_kind().to_owned(),
            endpoint: Some(config.endpoint),
            traces_endpoint: None,
            headers: Some(config.otlp_auth_headers),
            owner: Some(ExporterOwner::Levo),
            options: None,
            use_simple_processor: None,
            requires_headers: false,
        });
        Ok(settings)
    }
}

mod tests;
