use std::sync::Arc;

use betterlite_azure::azure_certificate::credential::{CertificateCredential, CertificateInput};
use betterlite_azure::azure_client_secret::transport::Clock as TokenClock;
use betterlite_azure::azure_client_secret::transport::Transport as ClientSecretTransport;
use betterlite_azure::azure_default_chain::grant::{CertificateRecord, GrantError};
use betterlite_azure::azure_default_chain::native::NativeGrant;

use super::super::azure_http::AzureIo;
use super::super::azure_production::SdkAzureGrants;
use super::super::azure_runtime::ProductionHost;

pub(super) fn certificate_grant<'grants, H: AzureIo>(
    grants: &'grants SdkAzureGrants<H>,
    record: &CertificateRecord,
) -> Result<NativeGrant<'grants, ProductionHost>, GrantError> {
    let input = CertificateInput {
        tenant_id: record.tenant_id.clone(),
        client_id: record.client_id.clone(),
        scope: Some(record.scope.clone()),
        certificate_path: Some(record.certificate_path.clone()),
        certificate_data: None,
        password: record.password.clone(),
        send_certificate_chain: record.send_certificate_chain,
        authority_host: grants.authority_host.clone(),
    };
    let transport: Arc<dyn ClientSecretTransport> = Arc::clone(&grants.http);
    let clock: Arc<dyn TokenClock> = Arc::clone(&grants.clock);
    let ids = Arc::clone(&grants.ids);
    let credential = CertificateCredential::new(input, &grants.files, transport, clock, ids)
        .map_err(|error| GrantError {
            credential: "CertificateCredential",
            message: error.to_string(),
        })?;
    Ok(NativeGrant::Certificate(credential))
}
