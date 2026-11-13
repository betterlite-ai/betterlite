use secrecy::{ExposeSecret, SecretString};

use betterlite_config::loader::EnvironmentLookup;

pub(crate) const REDACTED: &str = "[redacted]";

#[derive(Debug, Clone, Default)]
pub struct HostedEnvironment {
    pub identity_endpoint: Option<String>,
    pub identity_header: Option<SecretString>,
    pub identity_server_thumbprint: Option<String>,
    pub imds_endpoint: Option<String>,
    pub msi_endpoint: Option<String>,
    pub msi_secret: Option<SecretString>,
}

impl HostedEnvironment {
    #[must_use]
    pub fn from_environment<L>(lookup: &L) -> Self
    where
        L: EnvironmentLookup,
    {
        Self {
            identity_endpoint: lookup.lookup("IDENTITY_ENDPOINT"),
            identity_header: lookup.lookup("IDENTITY_HEADER").map(SecretString::from),
            identity_server_thumbprint: lookup.lookup("IDENTITY_SERVER_THUMBPRINT"),
            imds_endpoint: lookup.lookup("IMDS_ENDPOINT"),
            msi_endpoint: lookup.lookup("MSI_ENDPOINT"),
            msi_secret: lookup.lookup("MSI_SECRET").map(SecretString::from),
        }
    }

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
