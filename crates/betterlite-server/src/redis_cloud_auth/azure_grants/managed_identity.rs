use betterlite_azure::azure_arc_identity::ArcTransport;
use betterlite_azure::azure_default_chain::grant::{GrantError, ManagedIdentityRecord};
use betterlite_azure::azure_default_chain::native::NativeGrant;
use betterlite_azure::azure_default_chain::native::managed::ManagedIdentityGrant;
use betterlite_azure::azure_hosted_identity::request::{HostedIo, HostedTransport};
use betterlite_azure::azure_managed_identity::imds::ImdsTransport;
use betterlite_azure::azure_managed_identity::runtime::{
    ImdsIo, ManagedIdentityIo, TokenFiles, WorkloadIo,
};

use super::super::azure_http::AzureIo;
use super::super::azure_production::SdkAzureGrants;
use super::super::azure_runtime::ProductionHost;

pub(super) fn managed_identity_grant<'grants, H: AzureIo>(
    grants: &'grants SdkAzureGrants<H>,
    record: &ManagedIdentityRecord,
) -> Result<NativeGrant<'grants, ProductionHost>, GrantError> {
    let imds: &dyn ImdsTransport = &*grants.http;
    let hosted: &dyn HostedTransport = &*grants.http;
    let arc: &dyn ArcTransport = &*grants.http;
    let files: &dyn TokenFiles = &grants.token_files;
    let io = ManagedIdentityIo {
        imds: ImdsIo {
            transport: imds,
            scheduler: &grants.scheduler,
            ids: &*grants.ids,
            clock: &*grants.clock,
        },
        hosted: HostedIo {
            transport: hosted,
            clock: &*grants.clock,
            redactor: &grants.redactor,
        },
        arc,
        workload: WorkloadIo {
            exchange: &grants.exchange,
            files,
        },
    };
    let credential = ManagedIdentityGrant {
        environment: &grants.environment,
        host: &grants.host,
        io,
        client_id: record.client_id.clone(),
        exclude_workload_identity: record.exclude_workload_identity,
        imds_probe: record.imds_probe,
        placement: record.placement,
        scope: record.scope.clone(),
    };
    Ok(NativeGrant::ManagedIdentity(credential))
}
