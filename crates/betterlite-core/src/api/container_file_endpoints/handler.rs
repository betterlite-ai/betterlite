use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource,
};
use betterlite_openai::containers::container_file_http::{
    ContainerFileHttpCall, ContainerFileUpload, GenericContainerHandler, PreCallHook, ReplyResult,
};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

use crate::containers::container_request_parameters::ContainerFieldTypeError;

// the handler kwargs this module prepares: the catalog values, the file, and the caller's headers and query
pub struct ContainerFileHandlerCall<'call> {
    pub endpoint_name: &'call str,
    pub config: &'call dyn BaseContainerConfig,
    pub params: &'call Object,
    pub logging: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_query: Option<&'call IndexMap<String, ParamValue>>,
    pub timeout: Duration,
    pub values: &'call BTreeMap<String, ParamValue>,
    pub file: Option<&'call ContainerFileUpload>,
}

// the two forms of B3363's handler, which the blocking and async operations call
pub trait ContainerFileHandler: Send + Sync {
    fn handle(&self, call: &ContainerFileHandlerCall<'_>) -> ReplyResult;

    fn ahandle(
        &self,
        call: &ContainerFileHandlerCall<'_>,
    ) -> impl Future<Output = ReplyResult> + Send;
}

// B3361's get_provider_container_config, which answers None for a provider it has no container config for
pub trait ContainerConfigLookup: Send + Sync {
    fn container_config(&self, provider: ProviderId) -> Option<&dyn BaseContainerConfig>;
}

// what B3361's decode_managed_container_id_for_request answers, with the params it updated
pub struct DecodedContainerCall {
    pub container_id: String,
    pub custom_llm_provider: String,
    pub litellm_params: Object,
}

// B3361's decode_managed_container_id_for_request, which only touches a managed container id
pub trait ContainerIdDecoder: Send + Sync {
    fn decode(
        &self,
        container_id: &str,
        provider: &str,
        params: &Object,
    ) -> Result<DecodedContainerCall, ContainerFieldTypeError>;
}

impl<Source, Hook> ContainerFileHandler for GenericContainerHandler<Source, Hook>
where
    Source: ClientSource<Blocking: BlockingClient, Asynchronous: AsyncClient>,
    Hook: PreCallHook,
{
    fn handle(&self, call: &ContainerFileHandlerCall<'_>) -> ReplyResult {
        Self::handle(self, &http_call::<Source>(call))
    }

    async fn ahandle(&self, call: &ContainerFileHandlerCall<'_>) -> ReplyResult {
        Self::ahandle(self, &http_call::<Source>(call)).await
    }
}

// the handler builds its own client, since this module owns no transport
const fn http_call<'call, Source>(
    call: &'call ContainerFileHandlerCall<'call>,
) -> ContainerFileHttpCall<'call, Source>
where
    Source: ClientSource,
{
    ContainerFileHttpCall {
        endpoint_name: call.endpoint_name,
        config: call.config,
        litellm_params: call.params,
        logging: call.logging,
        extra_headers: call.extra_headers,
        extra_query: call.extra_query,
        timeout: Some(call.timeout),
        values: call.values,
        file: call.file,
        blocking_client: None,
        async_client: None,
    }
}
