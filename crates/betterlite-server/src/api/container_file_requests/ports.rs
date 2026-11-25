use async_trait::async_trait;
use betterlite_core::api::container_ownership::access::assert_container_access;
use betterlite_core::api::container_ownership::errors::ContainerOwnershipError;
use betterlite_core::api::container_ownership::forwarding::forwarding_params as get_forwarding_params;
use betterlite_core::api::container_ownership::repository::ContainerObjectTable;
use betterlite_core::api::container_ownership::{
    ContainerCaller, ContainerForwardingParams, ContainerIdentity, ContainerOwnership,
};
use betterlite_error::proxy_exception::ProxyException;

use super::{ContainerProcessingReply, ContainerRequestData, RequestFailure};
use crate::auth::auth_identity::AuthContext;
use crate::pre_call::failure::PreCallFailure;

// B3362's ownership service, injected so the processors run without a database
#[async_trait]
pub trait ContainerAccess: Send + Sync {
    async fn check_access(
        &self,
        container_id: &str,
        caller: &AuthContext,
        provider: &str,
    ) -> Result<ContainerIdentity, ContainerOwnershipError>;

    async fn forwarding_params(
        &self,
        container_id: &str,
        original_container_id: &str,
        provider: &str,
    ) -> Result<ContainerForwardingParams, ContainerOwnershipError>;
}

// B004's common request processing and its exception handling, supplied at wiring time
#[async_trait]
pub trait RequestProcessing: Send + Sync {
    async fn process(
        &self,
        data: &ContainerRequestData,
        route_type: &str,
        caller: &AuthContext,
    ) -> Result<ContainerProcessingReply, PreCallFailure>;

    async fn handle_exception(
        &self,
        failure: RequestFailure,
        caller: &AuthContext,
        data: &ContainerRequestData,
    ) -> ProxyException;
}

// The production ContainerAccess: B3362's ownership functions behind the trait the processors take
pub struct OwnershipAccess<Rows> {
    pub ownership: ContainerOwnership<Rows>,
}

#[async_trait]
impl<Rows> ContainerAccess for OwnershipAccess<Rows>
where
    Rows: ContainerObjectTable,
{
    async fn check_access(
        &self,
        container_id: &str,
        caller: &AuthContext,
        provider: &str,
    ) -> Result<ContainerIdentity, ContainerOwnershipError> {
        let container_caller = ContainerCaller {
            principal: Some(&caller.principal),
            custom_llm_provider: provider,
        };
        assert_container_access(&self.ownership, container_id, container_caller).await
    }

    async fn forwarding_params(
        &self,
        container_id: &str,
        original_container_id: &str,
        provider: &str,
    ) -> Result<ContainerForwardingParams, ContainerOwnershipError> {
        let identity = ContainerIdentity {
            original_container_id: original_container_id.to_owned(),
            custom_llm_provider: provider.to_owned(),
        };
        get_forwarding_params(&self.ownership, container_id, &identity).await
    }
}
