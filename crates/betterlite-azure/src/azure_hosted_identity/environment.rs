use secrecy::{ExposeSecret as _, SecretString};

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
    pub pod_identity_authority_host: Option<String>,
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
            pod_identity_authority_host: lookup.lookup("AZURE_POD_IDENTITY_AUTHORITY_HOST"),
        }
    }

    #[must_use]
    pub fn redact_known(&self, text: &str) -> String {
        let mut output = text.to_owned();
        for secret in [self.identity_header.as_ref(), self.msi_secret.as_ref()]
            .into_iter()
            .flatten()
        {
            let value = secret.expose_secret();
            if !value.is_empty() {
                output = output.replace(value, REDACTED);
            }
        }
        output
    }
}
