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
use betterlite_openai::containers::container_file_http::transport::ClientSource;
use betterlite_openai::openai_base_config::Object;

use crate::api::container_collection_http::{
    CollectionBase, CollectionTransport, ContainerCollectionError,
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
    pub timeout: Duration,
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

// blocking_client and async_client stay None, since the handler asks the source for the client its form needs
impl HandlerCallBase<'_> {
    const fn collection<'call, Source>(
        &'call self,
        source: &'call Source,
        hook: &'call dyn PreCallHook,
    ) -> CollectionBase<'call, Source>
    where
        Source: ClientSource,
    {
        CollectionBase {
            config: self.config,
            litellm_params: self.litellm_params,
            logging: self.logging,
            extra_headers: self.extra_headers,
            timeout: Some(self.timeout),
            source,
            blocking: None,
            asynchronous: None,
            hook,
        }
    }
}

// the two operations differ only in the named fields their collection call adds to the base, so one template spells them
macro_rules! collection_call {
    (create $build:ident, $call:ident, $collection:ident) => {
        const fn $build<'call, Source>(
            source: &'call Source,
            hook: &'call dyn PreCallHook,
            call: &'call $call<'call>,
        ) -> $collection<'call, Source>
        where
            Source: ClientSource,
        {
            $collection {
                base: call.base.collection(source, hook),
                name: call.name,
                params: call.params,
            }
        }
    };
    (list $build:ident, $call:ident, $collection:ident) => {
        const fn $build<'call, Source>(
            source: &'call Source,
            hook: &'call dyn PreCallHook,
            call: &'call $call<'call>,
        ) -> $collection<'call, Source>
        where
            Source: ClientSource,
        {
            $collection {
                base: call.base.collection(source, hook),
                query: ContainerListQuery {
                    after: call.after,
                    limit: call.limit,
                    order: call.order,
                    extra_query: call.extra_query,
                },
            }
        }
    };
}

collection_call!(create collection_create, CreateHandlerCall, CollectionCreate);
collection_call!(list collection_list, ListHandlerCall, CollectionList);

// each trait method builds its collection call and hands it to B3364's handler of the same form
macro_rules! collection_handler_methods {
    ($method:ident, $a_method:ident, $call:ident, $reply:ty, $build:ident, $blocking:path, $awaiting:path) => {
        fn $method(&self, call: &$call<'_>) -> Result<$reply, ContainerCollectionError> {
            $blocking(&$build(&self.source, &self.hook, call))
        }

        fn $a_method(
            &self,
            call: &$call<'_>,
        ) -> impl Future<Output = Result<$reply, ContainerCollectionError>> + Send {
            async move {
                let collection = $build(&self.source, &self.hook, call);
                $awaiting(&collection).await
            }
        }
    };
}

impl<Source, Hook> ContainerCollectionHandler for GenericCollectionHandler<Source, Hook>
where
    Source: ClientSource<Blocking: CollectionTransport, Asynchronous: CollectionTransport>,
    Hook: PreCallHook,
{
    collection_handler_methods!(
        create,
        acreate,
        CreateHandlerCall,
        ContainerObject,
        collection_create,
        container_create_handler,
        acontainer_create_handler
    );

    collection_handler_methods!(
        list,
        alist,
        ListHandlerCall,
        ContainerListResponse,
        collection_list,
        container_list_handler,
        acontainer_list_handler
    );
}
