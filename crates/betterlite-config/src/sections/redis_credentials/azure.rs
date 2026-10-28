use secrecy::{ExposeSecret as _, SecretString};

use crate::loader::EnvironmentLookup;

use super::RawSettings;

#[derive(Debug, Clone)]
pub struct AzureSettings {
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

pub(super) fn resolve_azure<L>(settings: &RawSettings, lookup: &L) -> AzureSettings
where
    L: EnvironmentLookup,
{
    let client_id = settings
        .azure_client_id
        .clone()
        .or_else(|| lookup.lookup("REDIS_AZURE_CLIENT_ID"))
        .or_else(|| lookup.lookup("AZURE_CLIENT_ID"));
    let tenant_id = settings
        .azure_tenant_id
        .clone()
        .or_else(|| lookup.lookup("REDIS_AZURE_TENANT_ID"))
        .or_else(|| lookup.lookup("AZURE_TENANT_ID"));
    let client_secret = settings
        .azure_client_secret
        .clone()
        .map(|value| value.expose_secret().to_owned())
        .or_else(|| lookup.lookup("REDIS_AZURE_CLIENT_SECRET"))
        .or_else(|| lookup.lookup("AZURE_CLIENT_SECRET"));
    AzureSettings {
        credential: azure_credential(
            client_id.as_deref(),
            tenant_id.as_deref(),
            client_secret.as_deref(),
        ),
        client_id: client_id.map(SecretString::from),
        tenant_id: tenant_id.map(SecretString::from),
        client_secret: client_secret.map(SecretString::from),
    }
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
