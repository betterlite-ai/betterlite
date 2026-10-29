use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, serde::Deserialize)]
#[rustfmt::skip]
pub struct Raw { #[serde(default)] pub litellm_settings: Global }

#[derive(Debug, Default, serde::Deserialize)]
#[rustfmt::skip]
pub struct Global { #[serde(default)] pub network_mock: bool }

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[rustfmt::skip]
pub struct Settings { pub network_mock: bool }

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            network_mock: self.litellm_settings.network_mock,
        })
    }
}

#[path = "openai_mock_transport/tests.rs"]
mod openai_mock_transport_tests;
