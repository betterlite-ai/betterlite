use betterlite_config::loader::EnvironmentLookup;

use crate::otel::config::Settings;
use crate::otel::presets::base::{Preset, PresetError};
use crate::otel::presets::utils::ensure_mappers;

pub struct LangtracePreset;

impl Preset for LangtracePreset {
    fn build(
        &self,
        _environment: &dyn EnvironmentLookup,
        base: Settings,
        _allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError> {
        let mut settings = base;
        settings.mapper_names = ensure_mappers(&settings.mapper_names, &["langtrace"]);
        Ok(settings)
    }
}

mod tests;
