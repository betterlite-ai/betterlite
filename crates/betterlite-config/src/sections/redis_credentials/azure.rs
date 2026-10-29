use secrecy::SecretString;

use crate::loader::EnvironmentLookup;

use super::RawSettings;

#[derive(Debug, Clone)]
pub struct AzureSettings {
    pub client_id: Option<SecretString>,
    pub tenant_id: Option<SecretString>,
    pub client_secret: Option<SecretString>,
    pub username: Option<String>,
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
        .or_else(|| {
            lookup
                .lookup("REDIS_AZURE_CLIENT_SECRET")
                .map(SecretString::from)
        })
        .or_else(|| lookup.lookup("AZURE_CLIENT_SECRET").map(SecretString::from));
    let username = lookup.lookup("REDIS_USERNAME");
    let credential = if client_id.is_some() && tenant_id.is_some() && client_secret.is_some() {
        AzureCredential::ClientSecret
    } else if client_id.is_some() {
        AzureCredential::ManagedIdentity
    } else {
        AzureCredential::Default
    };
    AzureSettings {
        credential,
        client_id: client_id.map(SecretString::from),
        tenant_id: tenant_id.map(SecretString::from),
        client_secret,
        username,
    }
}
