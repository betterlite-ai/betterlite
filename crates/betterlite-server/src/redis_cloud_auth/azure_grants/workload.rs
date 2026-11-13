use betterlite_azure::azure_default_chain::grant::{GrantError, WorkloadRecord};
use betterlite_azure::azure_default_chain::native::NativeGrant;
use betterlite_azure::azure_default_chain::native::workload::WorkloadGrant;
use betterlite_azure::azure_managed_identity::selection::WorkloadSelection;

use super::super::azure_http::AzureIo;
use super::super::azure_production::SdkAzureGrants;
use super::super::azure_runtime::ProductionHost;

pub(super) fn workload_grant<'grants, H: AzureIo>(
    grants: &'grants SdkAzureGrants<H>,
    record: &WorkloadRecord,
) -> Result<NativeGrant<'grants, ProductionHost>, GrantError> {
    let selection = WorkloadSelection {
        client_id: record.client_id.clone(),
        tenant_id: record.tenant_id.clone(),
        token_file: record.token_file.clone(),
    };
    let grant = WorkloadGrant {
        exchange: &grants.exchange,
        files: &grants.token_files,
        selection,
        scope: record.scope.clone(),
    };
    Ok(NativeGrant::Workload(grant))
}
