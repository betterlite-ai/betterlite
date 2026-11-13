use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    pub settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct RawSettings {
    #[serde(default)]
    pub disable_aiohttp_transport: bool,
}

// Python picks between aiohttp and httpx with this flag; betterlite has one reqwest engine, so it is read and never acted on
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub disable_aiohttp_transport: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let text = lookup
            .lookup("DISABLE_AIOHTTP_TRANSPORT")
            .unwrap_or_default();
        let from_environment = betterlite_env::flag("DISABLE_AIOHTTP_TRANSPORT", &text)?;
        Ok(Settings {
            disable_aiohttp_transport: self.settings.disable_aiohttp_transport
                || from_environment.unwrap_or(false),
        })
    }
}
