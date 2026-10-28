use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

use super::{RawSettings, choose};

#[derive(Debug, Clone)]
pub struct AzureSettings {
    pub token: Option<SecretString>,
    pub client_id: Option<SecretString>,
    pub tenant_id: Option<SecretString>,
    pub client_secret: Option<SecretString>,
    pub credential: AzureCredential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureCredential {
    Default,
    ManagedIdentity,
    ClientSecret,
}

pub(super) fn resolve_azure(
    settings: &RawSettings,
    lookup: &impl EnvironmentLookup,
) -> Option<AzureSettings> {
    let token = choose(
        settings.azure_redis_ad_token.clone(),
        lookup,
        "REDIS_AZURE_AD_TOKEN",
    );
    let client_id = choose(settings.azure_client_id.clone(), lookup, "AZURE_CLIENT_ID");
    let tenant_id = choose(settings.azure_tenant_id.clone(), lookup, "AZURE_TENANT_ID");
    let client_secret = choose(
        settings.azure_client_secret.clone(),
        lookup,
        "AZURE_CLIENT_SECRET",
    );
    (token.is_some() || client_id.is_some() || tenant_id.is_some() || client_secret.is_some())
        .then(|| AzureSettings {
            credential: azure_credential(
                client_id.as_deref(),
                tenant_id.as_deref(),
                client_secret.as_deref(),
            ),
            token: token.map(|value| SecretString::new(Box::<str>::from(value))),
            client_id: client_id.map(|value| SecretString::new(Box::<str>::from(value))),
            tenant_id: tenant_id.map(|value| SecretString::new(Box::<str>::from(value))),
            client_secret: client_secret.map(|value| SecretString::new(Box::<str>::from(value))),
        })
}

const fn azure_credential(
    client_id: Option<&str>,
    tenant_id: Option<&str>,
    secret: Option<&str>,
) -> AzureCredential {
    if client_id.is_some() && tenant_id.is_some() && secret.is_some() {
        AzureCredential::ClientSecret
    } else if client_id.is_some() {
        AzureCredential::ManagedIdentity
    } else {
        AzureCredential::Default
    }
}
