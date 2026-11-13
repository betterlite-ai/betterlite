use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const DEFAULT_AUTH_URL: &str = "https://ngw.devices.sberbank.ru:9443/api/v2/oauth";
pub const DEFAULT_SCOPE: &str = "GIGACHAT_API_PERS";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub access_token: Option<SecretString>,
    pub credentials: Option<SecretString>,
    pub auth_url: String,
    pub scope: String,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let read = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Ok(Settings {
            access_token: read("GIGACHAT_ACCESS_TOKEN").map(SecretString::from),
            credentials: read("GIGACHAT_CREDENTIALS")
                .or_else(|| read("GIGACHAT_API_KEY"))
                .map(SecretString::from),
            auth_url: read("GIGACHAT_AUTH_URL").unwrap_or_else(|| DEFAULT_AUTH_URL.to_owned()),
            scope: read("GIGACHAT_SCOPE").unwrap_or_else(|| DEFAULT_SCOPE.to_owned()),
        })
    }
}

mod tests;
