use secrecy::{ExposeSecret, SecretString};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const REDACTED: &str = "[redacted]";

// a map-shaped Raw, not a unit struct, so serde still accepts (and ignores) the document's keys
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

// every input stays Option: the SDK tells an absent variable from an explicitly empty one
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub identity_endpoint: Option<String>,
    pub identity_header: Option<SecretString>,
    pub identity_server_thumbprint: Option<String>,
    pub imds_endpoint: Option<String>,
    pub msi_endpoint: Option<String>,
    pub msi_secret: Option<SecretString>,
    pub pod_identity_authority_host: Option<String>,
}

impl Settings {
    #[must_use]
    pub fn redact_known(&self, text: &str) -> String {
        [self.identity_header.as_ref(), self.msi_secret.as_ref()]
            .into_iter()
            .flatten()
            .map(ExposeSecret::expose_secret)
            .filter(|value| !value.is_empty())
            .fold(text.to_owned(), |output, value| {
                output.replace(value, REDACTED)
            })
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            identity_endpoint: lookup.lookup("IDENTITY_ENDPOINT"),
            identity_header: lookup.lookup("IDENTITY_HEADER").map(SecretString::from),
            identity_server_thumbprint: lookup.lookup("IDENTITY_SERVER_THUMBPRINT"),
            imds_endpoint: lookup.lookup("IMDS_ENDPOINT"),
            msi_endpoint: lookup.lookup("MSI_ENDPOINT"),
            msi_secret: lookup.lookup("MSI_SECRET").map(SecretString::from),
            pod_identity_authority_host: lookup.lookup("AZURE_POD_IDENTITY_AUTHORITY_HOST"),
        })
    }
}
