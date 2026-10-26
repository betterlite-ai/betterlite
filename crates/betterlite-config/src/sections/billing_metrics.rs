use std::fs::File;
use std::path::PathBuf;

use secrecy::{ExposeSecret as _, SecretString};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod credentials;
mod tests;

pub const ENDPOINT_ENV: &str = "LITELLM_BILLING_METRICS_ENDPOINT";
pub const CLIENT_CERT_ENV: &str = "LITELLM_BILLING_METRICS_CLIENT_CERT";
pub const CLIENT_KEY_ENV: &str = "LITELLM_BILLING_METRICS_CLIENT_KEY";
pub const CA_CERT_ENV: &str = "LITELLM_BILLING_METRICS_CA_CERT";
pub const EXPORT_INTERVAL_ENV: &str = "LITELLM_BILLING_METRICS_EXPORT_INTERVAL_MS";
pub const DEFAULT_EXPORT_INTERVAL_MS: i64 = 60_000;
pub const METRICS_PATH: &str = "/v1/metrics";
// Python's tempfile.gettempdir reads these in this order before falling back to /tmp
const TEMP_ROOT_ENV: [&str; 3] = ["TMPDIR", "TEMP", "TMP"];

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub endpoint: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<SecretString>,
    pub ca_cert: Option<String>,
    // LiteLLM keeps any integer here and lets the exporter reject zero and negative values
    pub export_interval_ms: i64,
    pub temp_root: PathBuf,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            endpoint: None,
            client_cert: None,
            client_key: None,
            ca_cert: None,
            export_interval_ms: DEFAULT_EXPORT_INTERVAL_MS,
            temp_root: PathBuf::from("/tmp"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metering {
    pub endpoint: String,
    pub client_cert: PathBuf,
    pub client_key: PathBuf,
    pub ca_cert: Option<PathBuf>,
    pub export_interval_ms: i64,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        let temp_root = TEMP_ROOT_ENV
            .into_iter()
            .find_map(present)
            .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
        Ok(Settings {
            endpoint: present(ENDPOINT_ENV),
            client_cert: present(CLIENT_CERT_ENV),
            client_key: present(CLIENT_KEY_ENV).map(SecretString::from),
            ca_cert: present(CA_CERT_ENV),
            export_interval_ms: export_interval_ms(lookup.lookup(EXPORT_INTERVAL_ENV)),
            temp_root,
        })
    }
}

fn export_interval_ms(raw: Option<String>) -> i64 {
    let Some(value) = raw else {
        return DEFAULT_EXPORT_INTERVAL_MS;
    };
    value.trim().parse().unwrap_or_else(|_error| {
        tracing::warn!(
            "Invalid {EXPORT_INTERVAL_ENV}={value:?}, falling back to {DEFAULT_EXPORT_INTERVAL_MS} ms"
        );
        DEFAULT_EXPORT_INTERVAL_MS
    })
}

impl Settings {
    #[must_use]
    pub fn resolve(&self) -> Option<Metering> {
        let (Some(endpoint), Some(client_cert), Some(client_key)) =
            (&self.endpoint, &self.client_cert, &self.client_key)
        else {
            let missing: Vec<&str> = [
                (ENDPOINT_ENV, self.endpoint.is_none()),
                (CLIENT_CERT_ENV, self.client_cert.is_none()),
                (CLIENT_KEY_ENV, self.client_key.is_none()),
            ]
            .into_iter()
            .filter_map(|(variable, absent)| absent.then_some(variable))
            .collect();
            tracing::warn!(
                "Billing metrics disabled: licensed deployment missing config ({})",
                missing.join(", ")
            );
            return None;
        };
        let paths = self.credential_paths(client_cert, client_key)?;
        Some(Metering {
            endpoint: metrics_endpoint(endpoint),
            client_cert: paths.client_cert,
            client_key: paths.client_key,
            ca_cert: paths.ca_cert,
            export_interval_ms: self.export_interval_ms,
        })
    }

    fn credential_paths(
        &self,
        client_cert: &str,
        client_key: &SecretString,
    ) -> Option<credentials::CredentialPaths> {
        let paths = match credentials::resolve_paths(
            &self.temp_root,
            client_cert,
            client_key.expose_secret(),
            self.ca_cert.as_deref(),
        ) {
            Ok(paths) => paths,
            Err(error) => {
                tracing::warn!("Billing metrics disabled: {error}");
                return None;
            }
        };
        let unreadable: Vec<&str> = [
            (CLIENT_CERT_ENV, Some(&paths.client_cert)),
            (CLIENT_KEY_ENV, Some(&paths.client_key)),
            (CA_CERT_ENV, paths.ca_cert.as_ref()),
        ]
        .into_iter()
        .filter_map(|(variable, path)| {
            path.filter(|candidate| !(candidate.is_file() && File::open(candidate).is_ok()))
                .map(|_unreadable| variable)
        })
        .collect();
        if !unreadable.is_empty() {
            tracing::warn!(
                "Billing metrics disabled: {} did not resolve to a readable certificate file. Set each to a file path, or to inline PEM content beginning with '{}'.",
                unreadable.join(", "),
                credentials::PEM_PREFIX
            );
            return None;
        }
        Some(paths)
    }
}

fn metrics_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with(METRICS_PATH) {
        trimmed.to_owned()
    } else {
        format!("{trimmed}{METRICS_PATH}")
    }
}
