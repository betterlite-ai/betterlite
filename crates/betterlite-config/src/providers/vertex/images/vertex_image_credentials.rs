use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

// litellm_settings.vertex_credentials, the litellm.vertex_credentials global the image resolver reads
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    vertex_credentials: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub credentials: Option<ConfiguredCredentials>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let path = "litellm_settings.vertex_credentials";
        let credentials = self
            .litellm_settings
            .vertex_credentials
            .map(|configured| {
                let text = resolve_string(configured, path, lookup)?;
                ConfiguredCredentials::from_text(text).map_err(|_source| {
                    ConfigError::InvalidScalar {
                        path: path.to_owned(),
                        value: false,
                    }
                })
            })
            .transpose()?;
        Ok(Settings { credentials })
    }
}
