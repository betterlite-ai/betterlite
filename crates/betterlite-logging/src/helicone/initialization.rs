use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;

const DEFAULT_API_BASE: &str = "https://api.hconeai.com";

#[derive(Debug)]
pub struct Settings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

#[derive(Debug)]
pub struct HeliconeLogger {
    pub settings: Settings,
    pub client: ProxiedClient,
    pub timeout: Duration,
}

impl Settings {
    // An empty variable counts as unset, and one trailing slash is dropped from the base.
    #[must_use]
    pub fn from_environment(environment: &dyn EnvironmentLookup) -> Self {
        let api_key = environment
            .lookup("HELICONE_API_KEY")
            .filter(|key| !key.is_empty())
            .map(SecretString::from);
        let base = environment
            .lookup("HELICONE_API_BASE")
            .filter(|base| !base.is_empty())
            .map_or_else(
                || DEFAULT_API_BASE.to_owned(),
                |base| base.strip_suffix('/').unwrap_or(&base).to_owned(),
            );
        Self {
            api_key,
            api_base: base,
        }
    }
}

mod tests;
