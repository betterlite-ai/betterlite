use betterlite_azure::azure_client_secret::ClientSecretInput;
use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, GrantError};
use betterlite_azure::azure_default_chain::native::{NativeGrant, ResolvedValues};

use super::super::azure_http::AzureIo;
use super::super::azure_production::SdkAzureGrants;
use super::super::azure_runtime::ProductionHost;

pub(super) fn cloud_secret_grant<'grants, H: AzureIo>(
    grants: &'grants SdkAzureGrants<H>,
    record: &ClientSecretRecord,
) -> Result<NativeGrant<'grants, ProductionHost>, GrantError> {
    let ClientSecretRecord {
        tenant_id,
        client_id,
        client_secret,
        scope,
    } = record;
    let input = ClientSecretInput {
        tenant_id: tenant_id.clone(),
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
        scope: Some(scope.clone()),
    };
    let Ok(mut cache) = grants.client_secrets.lock() else {
        return Err(GrantError {
            credential: "ClientSecretCredential",
            message: "the credential cache is poisoned".to_owned(),
        });
    };
    let provider = cache
        .provider(&input, &ResolvedValues, grants.authority_host.as_deref())
        .map_err(|error| GrantError {
            credential: "ClientSecretCredential",
            message: error.to_string(),
        })?;
    Ok(NativeGrant::ClientSecret(provider))
}
