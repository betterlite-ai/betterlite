use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::containers::container_file_http::transport::ClientSource;
use betterlite_openai::containers::container_file_http::{ContainerFileHttpError, PreCallHook};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

use crate::api::container_resource_http::{
    ContainerResourceCall, ContainerResourceHandler, ContainerResourceHttpError, DeleteResult,
    Handled, RetrieveResult,
};

// one retrieve or delete call as B3365's handler takes it, with no caller-supplied client
pub struct ContainerHandlerCall<'call> {
    pub container_id: &'call str,
    pub config: &'call dyn BaseContainerConfig,
    pub litellm_params: &'call Object,
    pub logging: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_query: Option<&'call IndexMap<String, ParamValue>>,
    pub timeout: Option<Duration>,
}

// the four forms of B3365's handler that the operations call
pub trait ContainerHandler: Send + Sync {
    fn retrieve(&self, call: &ContainerHandlerCall<'_>) -> RetrieveResult;

    fn delete(&self, call: &ContainerHandlerCall<'_>) -> DeleteResult;

    fn aretrieve(
        &self,
        call: &ContainerHandlerCall<'_>,
    ) -> impl Future<Output = RetrieveResult> + Send;

    fn adelete(&self, call: &ContainerHandlerCall<'_>)
    -> impl Future<Output = DeleteResult> + Send;
}

impl<Source, Hook> ContainerHandler for ContainerResourceHandler<Source, Hook>
where
    Source: ClientSource,
    Hook: PreCallHook,
{
    fn retrieve(&self, call: &ContainerHandlerCall<'_>) -> RetrieveResult {
        match Self::container_retrieve_handler(self, &http_call::<Source>(call), false) {
            Ok(Handled::Done(object)) => Ok(object),
            Ok(Handled::Pending(_)) => Err(ContainerResourceHttpError::Call(
                ContainerFileHttpError::Suspended,
            )),
            Err(error) => Err(error),
        }
    }

    fn delete(&self, call: &ContainerHandlerCall<'_>) -> DeleteResult {
        match Self::container_delete_handler(self, &http_call::<Source>(call), false) {
            Ok(Handled::Done(deleted)) => Ok(deleted),
            Ok(Handled::Pending(_)) => Err(ContainerResourceHttpError::Call(
                ContainerFileHttpError::Suspended,
            )),
            Err(error) => Err(error),
        }
    }

    fn aretrieve(
        &self,
        call: &ContainerHandlerCall<'_>,
    ) -> impl Future<Output = RetrieveResult> + Send {
        let request = http_call::<Source>(call);
        async move { Self::acontainer_retrieve_handler(self, &request).await }
    }

    fn adelete(
        &self,
        call: &ContainerHandlerCall<'_>,
    ) -> impl Future<Output = DeleteResult> + Send {
        let request = http_call::<Source>(call);
        async move { Self::acontainer_delete_handler(self, &request).await }
    }
}

// the handler builds its own client, since this module owns no transport
const fn http_call<'call, Source: ClientSource>(
    call: &'call ContainerHandlerCall<'_>,
) -> ContainerResourceCall<'call, Source> {
    ContainerResourceCall {
        container_id: call.container_id,
        config: call.config,
        litellm_params: call.litellm_params,
        logging_obj: call.logging,
        extra_headers: call.extra_headers,
        extra_query: call.extra_query,
        timeout: call.timeout,
        blocking_client: None,
        async_client: None,
    }
}
