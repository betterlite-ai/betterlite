use crate::azure_arc_identity::ArcHost;
use crate::azure_managed_identity::runtime::TokenFiles;
use crate::azure_managed_identity::selection::WorkloadSelection;
use crate::azure_managed_identity::workload;
use crate::oidc_exchange::AzureOidcExchange;

use super::super::grant::{ChainToken, Grant, LinkError, WorkloadRecord};
use super::NativeGrants;

pub struct WorkloadGrant<'io> {
    pub exchange: &'io AzureOidcExchange,
    pub files: &'io dyn TokenFiles,
    pub selection: WorkloadSelection,
    pub scope: String,
}

impl WorkloadGrant<'_> {
    pub fn from_record<'grants, 'io, H: ArcHost>(
        record: &WorkloadRecord,
        source: &'grants NativeGrants<'io, H>,
    ) -> WorkloadGrant<'grants>
    where
        'io: 'grants,
    {
        WorkloadGrant {
            exchange: source.exchange,
            files: source.token_files,
            selection: WorkloadSelection {
                client_id: record.client_id.clone(),
                tenant_id: record.tenant_id.clone(),
                token_file: record.token_file.clone(),
            },
            scope: record.scope.clone(),
        }
    }
}

impl Grant for WorkloadGrant<'_> {
    fn token(&self) -> Result<ChainToken, LinkError> {
        workload::acquire(self.exchange, self.files, &self.selection, &self.scope)
            .map(|token| ChainToken {
                token,
                expires_on: None,
                token_type: None,
                refresh_on: None,
            })
            .map_err(|error| LinkError::Authentication(error.to_string()))
    }
}
