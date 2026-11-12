use secrecy::SecretString;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// LiteLLM resolves AZURE_CREDENTIAL with getattr, so only these names reach a native credential
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AzureCredentialType {
    ClientSecretCredential,
    CertificateCredential,
    UsernamePasswordCredential,
    ManagedIdentityCredential,
    WorkloadIdentityCredential,
    DefaultAzureCredential,
    DeploymentIdentityCredential,
}

const CREDENTIALS: [(&str, AzureCredentialType); 7] = [
    (
        "ClientSecretCredential",
        AzureCredentialType::ClientSecretCredential,
    ),
    (
        "CertificateCredential",
        AzureCredentialType::CertificateCredential,
    ),
    (
        "UsernamePasswordCredential",
        AzureCredentialType::UsernamePasswordCredential,
    ),
    (
        "ManagedIdentityCredential",
        AzureCredentialType::ManagedIdentityCredential,
    ),
    (
        "WorkloadIdentityCredential",
        AzureCredentialType::WorkloadIdentityCredential,
    ),
    (
        "DefaultAzureCredential",
        AzureCredentialType::DefaultAzureCredential,
    ),
    (
        "DeploymentIdentityCredential",
        AzureCredentialType::DeploymentIdentityCredential,
    ),
];

// the developer credentials DefaultAzureCredential would build cannot run without a subprocess
const DEVELOPER_FILTERS: [&str; 6] = [
    "dev",
    "azureclicredential",
    "azuredeveloperclicredential",
    "azurepowershellcredential",
    "interactivebrowsercredential",
    "visualstudiocodecredential",
];

impl AzureCredentialType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClientSecretCredential => "ClientSecretCredential",
            Self::CertificateCredential => "CertificateCredential",
            Self::UsernamePasswordCredential => "UsernamePasswordCredential",
            Self::ManagedIdentityCredential => "ManagedIdentityCredential",
            Self::WorkloadIdentityCredential => "WorkloadIdentityCredential",
            Self::DefaultAzureCredential => "DefaultAzureCredential",
            Self::DeploymentIdentityCredential => "DeploymentIdentityCredential",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        CREDENTIALS
            .iter()
            .find_map(|(name, credential)| (*name == value).then_some(*credential))
    }
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub azure_credential: Option<AzureCredentialType>,
    pub azure_federated_token_file: Option<String>,
    pub azure_certificate_path: Option<String>,
    pub azure_certificate_password: Option<SecretString>,
    pub azure_token_credentials: String,
    pub azure_client_certificate_path: Option<String>,
    pub client_certificate_password: Option<SecretString>,
    pub client_send_certificate_chain: bool,
}

impl Settings {
    #[must_use]
    pub fn redact_known(&self, text: &str) -> String {
        [
            self.azure_certificate_password.as_ref(),
            self.client_certificate_password.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(secrecy::ExposeSecret::expose_secret)
        .filter(|value| !value.is_empty())
        .fold(text.to_owned(), |output, value| {
            output.replace(value, "[redacted]")
        })
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // an empty name is Python's falsy value and falls through to inference
        let azure_credential = match lookup.lookup("AZURE_CREDENTIAL") {
            Some(value) if value.is_empty() => None,
            Some(value) => Some(AzureCredentialType::parse(&value).ok_or_else(|| {
                ConfigError::InvalidScalar {
                    path: format!("AZURE_CREDENTIAL={value}"),
                    value: false,
                }
            })?),
            None => None,
        };
        let azure_token_credentials = lookup
            .lookup("AZURE_TOKEN_CREDENTIALS")
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if DEVELOPER_FILTERS.contains(&azure_token_credentials.as_str()) {
            return Err(ConfigError::InvalidScalar {
                path: format!(
                    "AZURE_TOKEN_CREDENTIALS={azure_token_credentials}, a developer credential this native build does not support"
                ),
                value: false,
            });
        }
        Ok(Settings {
            azure_credential,
            azure_federated_token_file: lookup.lookup("AZURE_FEDERATED_TOKEN_FILE"),
            azure_certificate_path: lookup.lookup("AZURE_CERTIFICATE_PATH"),
            azure_certificate_password: lookup
                .lookup("AZURE_CERTIFICATE_PASSWORD")
                .map(SecretString::from),
            azure_token_credentials,
            azure_client_certificate_path: lookup.lookup("AZURE_CLIENT_CERTIFICATE_PATH"),
            client_certificate_password: lookup
                .lookup("AZURE_CLIENT_CERTIFICATE_PASSWORD")
                .map(SecretString::from),
            // the SDK takes Python bool of the raw string: any nonempty value, "false" included, is true
            client_send_certificate_chain: lookup
                .lookup("AZURE_CLIENT_SEND_CERTIFICATE_CHAIN")
                .is_some_and(|value| !value.is_empty()),
        })
    }
}
