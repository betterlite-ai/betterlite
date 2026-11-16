use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;

mod tests;

// the parts of main.py's embedding aliases that B1301 does not already read
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    api_base: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub project: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub api_base: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            project: read(lookup, &["VERTEX_PROJECT"]),
            credentials: parse_credentials(read(lookup, &["VERTEX_CREDENTIALS"]))?,
            api_base: match self.litellm_settings.api_base {
                Some(text) => Some(resolve_string(text, "litellm_settings.api_base", lookup)?),
                None => read(lookup, &["VERTEXAI_API_BASE", "VERTEX_API_BASE"]),
            },
        })
    }
}

// an empty environment variable reads as unset everywhere LiteLLM reads one
fn read(lookup: &dyn EnvironmentLookup, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| lookup.lookup(name).filter(|text| !text.is_empty()))
}

fn parse_credentials(text: Option<String>) -> Result<Option<ConfiguredCredentials>, ConfigError> {
    let Some(credentials) = text else {
        return Ok(None);
    };
    ConfiguredCredentials::from_text(credentials)
        .map(Some)
        .map_err(|_source| ConfigError::InvalidScalar {
            path: "VERTEX_CREDENTIALS".to_owned(),
            value: false,
        })
}
