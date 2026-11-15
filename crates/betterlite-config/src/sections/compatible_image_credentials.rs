use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use secrecy::{ExposeSecret as _, SecretString};

pub const AIML_DEFAULT_API_BASE: &str = "https://api.aimlapi.com";
pub const COMETAPI_DEFAULT_API_BASE: &str = "https://api.cometapi.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum Host {
    #[serde(rename = "aiml")]
    Aiml,
    #[serde(rename = "cometapi")]
    CometApi,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub aiml_api_base: Option<String>,
    pub aiml_api_key: Option<SecretString>,
    pub cometapi_api_base: Option<String>,
    pub cometapi_api_key: Option<SecretString>,
}

impl Settings {
    // LiteLLM reads explicit, then the host's environment value, then the default, and an empty value falls through
    #[must_use]
    pub fn resolve(
        &self,
        host: Host,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (String, Option<SecretString>) {
        let (environment_base, environment_key, default_base) = match host {
            Host::Aiml => (
                self.aiml_api_base.as_deref(),
                self.aiml_api_key.as_ref(),
                AIML_DEFAULT_API_BASE,
            ),
            Host::CometApi => (
                self.cometapi_api_base.as_deref(),
                self.cometapi_api_key.as_ref(),
                COMETAPI_DEFAULT_API_BASE,
            ),
        };
        let base = api_base
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| environment_base.map(str::to_owned))
            .unwrap_or_else(|| default_base.to_owned());
        let key = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .cloned()
            .or_else(|| environment_key.cloned());
        (base, key)
    }
}

fn first_alias(values: [Option<String>; 2]) -> Option<String> {
    for value in values {
        if let Some(text) = value.filter(|text| !text.is_empty()) {
            return Some(text);
        }
    }
    None
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let aiml_api_key =
            first_alias([lookup.lookup("AIML_API_KEY"), lookup.lookup("AIMLAPI_KEY")]);
        let cometapi_api_key = first_alias([
            lookup.lookup("COMETAPI_KEY"),
            lookup.lookup("COMETAPI_API_KEY"),
        ]);
        let aiml_api_base = first_alias([lookup.lookup("AIML_API_BASE"), None]);
        let cometapi_api_base = first_alias([
            lookup.lookup("COMETAPI_BASE_URL"),
            lookup.lookup("COMETAPI_API_BASE"),
        ]);
        Ok(Settings {
            aiml_api_base,
            aiml_api_key: aiml_api_key.map(SecretString::from),
            cometapi_api_base,
            cometapi_api_key: cometapi_api_key.map(SecretString::from),
        })
    }
}
