use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_default_chain::{AzureCredentialType, Settings};
use secrecy::SecretString;

pub const DEFAULT_SCOPE: &str = "https://cognitiveservices.azure.com/.default";

#[derive(Debug, Clone, Default)]
pub struct ChainEnvironment {
    pub scope: Option<String>,
    pub credential: Option<AzureCredentialType>,
    pub client_id: Option<String>,
    pub tenant_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub username: Option<String>,
    pub password: Option<SecretString>,
    pub federated_token_file: Option<String>,
    pub certificate_path: Option<String>,
    pub certificate_password: Option<SecretString>,
    pub client_certificate_path: Option<String>,
    pub client_certificate_password: Option<SecretString>,
    pub client_send_certificate_chain: bool,
    pub token_credentials: String,
    pub authority_host: Option<String>,
}

impl ChainEnvironment {
    // an explicit empty scope is forwarded; only an absent one falls back to AZURE_SCOPE then the default
    #[must_use]
    pub fn resolved_scope(&self, explicit: Option<&str>) -> String {
        explicit.map_or_else(
            || {
                self.scope
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .unwrap_or(DEFAULT_SCOPE)
                    .to_owned()
            },
            str::to_owned,
        )
    }

    // the shared identity names stay with the lookup; the eight default-chain keys arrive parsed
    #[must_use]
    pub fn from_settings<L>(settings: &Settings, lookup: &L) -> Self
    where
        L: EnvironmentLookup,
    {
        Self {
            scope: lookup.lookup("AZURE_SCOPE"),
            credential: settings.azure_credential,
            client_id: lookup.lookup("AZURE_CLIENT_ID"),
            tenant_id: lookup.lookup("AZURE_TENANT_ID"),
            client_secret: lookup.lookup("AZURE_CLIENT_SECRET").map(SecretString::from),
            username: lookup.lookup("AZURE_USERNAME"),
            password: lookup.lookup("AZURE_PASSWORD").map(SecretString::from),
            federated_token_file: settings.azure_federated_token_file.clone(),
            certificate_path: settings.azure_certificate_path.clone(),
            certificate_password: settings.azure_certificate_password.clone(),
            client_certificate_path: settings.azure_client_certificate_path.clone(),
            client_certificate_password: settings.client_certificate_password.clone(),
            client_send_certificate_chain: settings.client_send_certificate_chain,
            token_credentials: settings.azure_token_credentials.clone(),
            authority_host: lookup.lookup("AZURE_AUTHORITY_HOST"),
        }
    }

    // infer_credential_type_from_environment tests truthiness, so an empty variable counts as unset
    #[must_use]
    pub fn infer_credential_type(&self) -> AzureCredentialType {
        let text = |value: &Option<String>| value.as_deref().is_some_and(|text| !text.is_empty());
        let secret = |value: &Option<SecretString>| {
            value
                .as_ref()
                .is_some_and(|secret| !secrecy::ExposeSecret::expose_secret(secret).is_empty())
        };
        if text(&self.client_id) && secret(&self.client_secret) && text(&self.tenant_id) {
            AzureCredentialType::ClientSecretCredential
        } else if text(&self.federated_token_file) {
            AzureCredentialType::DefaultAzureCredential
        } else if text(&self.client_id) {
            AzureCredentialType::ManagedIdentityCredential
        } else if secret(&self.certificate_password) || text(&self.certificate_path) {
            AzureCredentialType::CertificateCredential
        } else {
            AzureCredentialType::DefaultAzureCredential
        }
    }
}
