mod certificate;
mod client_secret;
mod managed_identity;
mod password;
mod workload;

use betterlite_azure::azure_default_chain::grant::{
    CertificateRecord, ClientSecretRecord, GrantError, Grants, ManagedIdentityRecord,
    UsernamePasswordRecord, WorkloadRecord,
};
use betterlite_azure::azure_default_chain::native::NativeGrant;

use super::azure_http::AzureIo;
use super::azure_production::SdkAzureGrants;
use super::azure_runtime::ProductionHost;

impl<H: AzureIo> Grants for SdkAzureGrants<H> {
    type Grant<'grants>
        = NativeGrant<'grants, ProductionHost>
    where
        Self: 'grants;

    fn client_secret<'grants>(
        &'grants self,
        record: &ClientSecretRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        client_secret::cloud_secret_grant(self, record)
    }

    fn certificate<'grants>(
        &'grants self,
        record: &CertificateRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        certificate::certificate_grant(self, record)
    }

    fn username_password<'grants>(
        &'grants self,
        record: &UsernamePasswordRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        password::password_grant(self, record)
    }

    fn workload<'grants>(
        &'grants self,
        record: &WorkloadRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        workload::workload_grant(self, record)
    }

    fn managed_identity<'grants>(
        &'grants self,
        record: &ManagedIdentityRecord,
    ) -> Result<Self::Grant<'grants>, GrantError> {
        managed_identity::managed_identity_grant(self, record)
    }
}
