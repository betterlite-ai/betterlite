use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::container_contracts::config::{
    BaseContainerConfig, ContainerListQuery,
};
use betterlite_openai::api::container_contracts::records::{
    ContainerListResponse, ContainerObject,
};
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::openai_base_config::Object;

use crate::api::container_collection_http::{
    CollectionBase, CollectionClientSource, ContainerCollectionError,
    ContainerCreateCall as CollectionCreate, ContainerListCall as CollectionList,
    acontainer_create_handler, acontainer_list_handler, container_create_handler,
    container_list_handler,
};

// the arguments both operations hand B3364's handlers, the named values beside them
pub struct HandlerCallBase<'call> {
    pub config: &'call dyn BaseContainerConfig,
    pub litellm_params: &'call Object,
    pub logging: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
}

pub struct CreateHandlerCall<'call> {
    pub base: HandlerCallBase<'call>,
    pub name: &'call str,
    pub params: &'call Object,
}

pub struct ListHandlerCall<'call> {
    pub base: HandlerCallBase<'call>,
    pub after: Option<&'call str>,
    pub limit: Option<i64>,
    pub order: Option<&'call str>,
    pub extra_query: Option<&'call Object>,
}

// B3364's container_create_handler, acontainer_create_handler, container_list_handler and acontainer_list_handler, as one seam
pub trait ContainerCollectionHandler: Send + Sync {
    fn create(
        &self,
        call: &CreateHandlerCall<'_>,
    ) -> Result<ContainerObject, ContainerCollectionError>;

    fn acreate(
        &self,
        call: &CreateHandlerCall<'_>,
    ) -> impl Future<Output = Result<ContainerObject, ContainerCollectionError>> + Send;

    fn list(
        &self,
        call: &ListHandlerCall<'_>,
    ) -> Result<ContainerListResponse, ContainerCollectionError>;

    fn alist(
        &self,
        call: &ListHandlerCall<'_>,
    ) -> impl Future<Output = Result<ContainerListResponse, ContainerCollectionError>> + Send;
}

// the transport and pre-call hook B3364's handlers build their client from and log through
pub struct GenericCollectionHandler<Source, Hook> {
    pub source: Source,
    pub hook: Hook,
}

impl<Source, Hook> ContainerCollectionHandler for GenericCollectionHandler<Source, Hook>
where
    Source: CollectionClientSource,
    Hook: PreCallHook,
{
    fn create(
        &self,
        call: &CreateHandlerCall<'_>,
    ) -> Result<ContainerObject, ContainerCollectionError> {
        container_create_handler(&collection_create(self, call))
    }

    fn acreate(
        &self,
        call: &CreateHandlerCall<'_>,
    ) -> impl Future<Output = Result<ContainerObject, ContainerCollectionError>> + Send {
        async move {
            let collection = collection_create(self, call);
            acontainer_create_handler(&collection).await
        }
    }

    fn list(
        &self,
        call: &ListHandlerCall<'_>,
    ) -> Result<ContainerListResponse, ContainerCollectionError> {
        container_list_handler(&collection_list(self, call))
    }

    fn alist(
        &self,
        call: &ListHandlerCall<'_>,
    ) -> impl Future<Output = Result<ContainerListResponse, ContainerCollectionError>> + Send {
        async move {
            let collection = collection_list(self, call);
            acontainer_list_handler(&collection).await
        }
    }
}

// blocking_client and async_client stay None, since the handler asks the source for the client its form needs
fn collection_base<'call, Source, Hook>(
    handler: &'call GenericCollectionHandler<Source, Hook>,
    base: &'call HandlerCallBase<'call>,
) -> CollectionBase<'call, Source>
where
    Source: CollectionClientSource,
    Hook: PreCallHook,
{
    CollectionBase {
        config: base.config,
        litellm_params: base.litellm_params,
        logging: base.logging,
        extra_headers: base.extra_headers,
        timeout: base.timeout,
        source: &handler.source,
        blocking: None,
        asynchronous: None,
        hook: &handler.hook,
    }
}

fn collection_create<'call, Source, Hook>(
    handler: &'call GenericCollectionHandler<Source, Hook>,
    call: &'call CreateHandlerCall<'call>,
) -> CollectionCreate<'call, Source>
where
    Source: CollectionClientSource,
    Hook: PreCallHook,
{
    CollectionCreate {
        base: collection_base(handler, &call.base),
        name: call.name,
        params: call.params,
    }
}

fn collection_list<'call, Source, Hook>(
    handler: &'call GenericCollectionHandler<Source, Hook>,
    call: &'call ListHandlerCall<'call>,
) -> CollectionList<'call, Source>
where
    Source: CollectionClientSource,
    Hook: PreCallHook,
{
    CollectionList {
        base: collection_base(handler, &call.base),
        query: ContainerListQuery {
            after: call.after,
            limit: call.limit,
            order: call.order,
            extra_query: call.extra_query,
        },
    }
}
