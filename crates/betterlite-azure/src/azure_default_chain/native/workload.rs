use crate::azure_managed_identity::runtime::TokenFiles;
use crate::azure_managed_identity::selection::WorkloadSelection;
use crate::azure_managed_identity::workload;
use crate::oidc_exchange::AzureOidcExchange;

use super::super::grant::{ChainToken, Grant, LinkError};

pub struct WorkloadGrant<'io> {
    pub exchange: &'io AzureOidcExchange,
    pub files: &'io dyn TokenFiles,
    pub selection: WorkloadSelection,
    pub scope: String,
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
