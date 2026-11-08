use betterlite_config::loader::EnvironmentLookup;

use crate::otel::config::Settings;
use crate::otel::integrations::levo_settings::LevoSettingsError;
use crate::otel::integrations::phoenix_settings::PhoenixSettingsError;
use crate::otel::integrations::weave_settings::WeaveSettingsError;

pub trait Preset {
    fn build(
        &self,
        environment: &dyn EnvironmentLookup,
        base: Settings,
        allow_missing_credentials: bool,
    ) -> Result<Settings, PresetError>;
}

#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error(transparent)]
    Phoenix(#[from] PhoenixSettingsError),
    #[error(transparent)]
    Levo(#[from] LevoSettingsError),
    #[error(transparent)]
    Weave(#[from] WeaveSettingsError),
}

mod tests;
