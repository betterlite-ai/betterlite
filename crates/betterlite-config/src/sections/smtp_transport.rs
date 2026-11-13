use std::time::Duration;

use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const DEFAULT_PORT: u16 = 587;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    _marker: Option<()>,
}

// SMTP_USE_SSL and SMTP_TLS compare to these exact strings in LiteLLM, so "true" leaves STARTTLS on
#[derive(Debug, Clone)]
pub struct Settings {
    pub host: Option<String>,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub sender_email: Option<String>,
    pub use_ssl: bool,
    pub tls: bool,
    pub timeout: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            host: None,
            port: DEFAULT_PORT,
            username: None,
            password: None,
            sender_email: None,
            use_ssl: false,
            tls: true,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let port = betterlite_env::number::<u16>(
            "SMTP_PORT",
            &lookup.lookup("SMTP_PORT").unwrap_or_default(),
        )?
        .unwrap_or(DEFAULT_PORT);
        let timeout = match betterlite_env::number::<f64>(
            "SMTP_TIMEOUT",
            &lookup.lookup("SMTP_TIMEOUT").unwrap_or_default(),
        )? {
            Some(seconds) => Duration::try_from_secs_f64(seconds).map_err(|_source| {
                betterlite_env::EnvError::Number {
                    name: "SMTP_TIMEOUT".to_owned(),
                }
            })?,
            None => DEFAULT_TIMEOUT,
        };
        Ok(Settings {
            host: lookup.lookup("SMTP_HOST").filter(|value| !value.is_empty()),
            port,
            username: lookup
                .lookup("SMTP_USERNAME")
                .filter(|value| !value.is_empty()),
            password: lookup
                .lookup("SMTP_PASSWORD")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            sender_email: lookup
                .lookup("SMTP_SENDER_EMAIL")
                .filter(|value| !value.is_empty()),
            use_ssl: lookup
                .lookup("SMTP_USE_SSL")
                .is_some_and(|value| value == "True"),
            tls: lookup
                .lookup("SMTP_TLS")
                .is_none_or(|value| value != "False"),
            timeout,
        })
    }
}
