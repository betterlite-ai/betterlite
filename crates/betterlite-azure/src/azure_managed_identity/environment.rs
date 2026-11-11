use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::azure_managed_identity::Settings;

use crate::azure_hosted_identity::environment::HostedEnvironment;

#[derive(Debug, Clone, Default)]
pub struct ManagedIdentityEnvironment {
    pub hosted: HostedEnvironment,
    pub pod_identity_authority_host: Option<String>,
    pub azure_authority_host: Option<String>,
    pub azure_tenant_id: Option<String>,
    pub azure_federated_token_file: Option<String>,
    pub azure_client_id: Option<String>,
}

impl ManagedIdentityEnvironment {
    #[must_use]
    pub fn from_lookup<L>(lookup: &L) -> Self
    where
        L: EnvironmentLookup,
    {
        Self {
            hosted: HostedEnvironment::from_environment(lookup),
            pod_identity_authority_host: lookup.lookup("AZURE_POD_IDENTITY_AUTHORITY_HOST"),
            azure_authority_host: lookup.lookup("AZURE_AUTHORITY_HOST"),
            azure_tenant_id: lookup.lookup("AZURE_TENANT_ID"),
            azure_federated_token_file: lookup.lookup("AZURE_FEDERATED_TOKEN_FILE"),
            azure_client_id: lookup.lookup("AZURE_CLIENT_ID"),
        }
    }

    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            hosted: HostedEnvironment {
                identity_endpoint: settings.identity_endpoint.clone(),
                identity_header: settings.identity_header.clone(),
                identity_server_thumbprint: settings.identity_server_thumbprint.clone(),
                imds_endpoint: settings.imds_endpoint.clone(),
                msi_endpoint: settings.msi_endpoint.clone(),
                msi_secret: settings.msi_secret.clone(),
            },
            pod_identity_authority_host: settings.pod_identity_authority_host.clone(),
            azure_authority_host: None,
            azure_tenant_id: None,
            azure_federated_token_file: None,
            azure_client_id: None,
        }
    }
}
