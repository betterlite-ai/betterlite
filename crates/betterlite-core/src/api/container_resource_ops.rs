pub mod error;
pub mod handler;
mod mock;
mod prepare;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::{
    ContainerObject, DeleteContainerResult,
};
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::error::{ContainerOpError, ContainerOpFailure, failure};
use self::handler::ContainerHandler;
use self::mock::{mock_delete, mock_retrieve};
use self::prepare::caller_provider;
use crate::api::container_file_endpoints::handler::{ContainerConfigLookup, ContainerIdDecoder};

const RETRIEVE_CALL_TYPE: &str = "retrieve_container";
const DELETE_CALL_TYPE: &str = "delete_container";

// the named arguments retrieve_container and delete_container take, which LiteLLM declares once and shares
pub struct ContainerOperationRequest<'request> {
    pub container_id: &'request str,
    // the declared default is 600 seconds; an explicit zero is LiteLLM's `timeout or DEFAULT_REQUEST_TIMEOUT`
    pub timeout: Option<Duration>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<&'request str>,
    pub api_version: Option<&'request str>,
    // the declared default is "openai"
    pub custom_llm_provider: Option<&'request str>,
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    pub extra_query: Option<&'request IndexMap<String, ParamValue>>,
    // accepted to match the signature; LiteLLM reads it only to pass None to the response encoder
    pub extra_body: Option<&'request IndexMap<String, JsonValue>>,
    pub litellm_call_id: Option<&'request str>,
    pub litellm_metadata: Option<&'request IndexMap<String, Json>>,
    pub mock_response: Option<&'request Json>,
}

// the collaborators the operations call: B3365's handler, B3361's decoder and config lookup, and B247's logging
pub struct ContainerOperations<'deps, Handler, Decoder, Configs> {
    pub handler: &'deps Handler,
    pub decoder: &'deps Decoder,
    pub configs: &'deps Configs,
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    // LiteLLM's request_timeout constant that REQUEST_TIMEOUT overrides, used only where LiteLLM reaches it
    pub request_timeout_seconds: f64,
}

impl<'deps, Handler, Decoder, Configs> ContainerOperations<'deps, Handler, Decoder, Configs>
where
    Handler: ContainerHandler,
    Decoder: ContainerIdDecoder,
    Configs: ContainerConfigLookup,
{
    // LiteLLM's retrieve_container: the mock short circuit, then the blocking handler and the id re-encoding
    pub fn retrieve_container(
        &self,
        request: &ContainerOperationRequest<'_>,
    ) -> Result<ContainerObject, ContainerOpFailure> {
        if let Some(object) = mock_retrieve(request.mock_response)
            .map_err(|error| failure(error, caller_provider(request), self.redactor))?
        {
            return Ok(object);
        }
        let prepared = self.prepare(request, RETRIEVE_CALL_TYPE)?;
        let call = prepared.handler_call(request, self.logging);
        let mut object = self.handler.retrieve(&call).map_err(|error| {
            failure(
                ContainerOpError::Handler(error),
                &prepared.provider,
                self.redactor,
            )
        })?;
        object.id = prepared.encoded_id(&object.id, request, self.redactor)?;
        Ok(object)
    }

    // LiteLLM's delete_container, with the same steps and the DeleteContainerResult mock
    pub fn delete_container(
        &self,
        request: &ContainerOperationRequest<'_>,
    ) -> Result<DeleteContainerResult, ContainerOpFailure> {
        if let Some(deleted) = mock_delete(request.mock_response)
            .map_err(|error| failure(error, caller_provider(request), self.redactor))?
        {
            return Ok(deleted);
        }
        let prepared = self.prepare(request, DELETE_CALL_TYPE)?;
        let call = prepared.handler_call(request, self.logging);
        let mut deleted = self.handler.delete(&call).map_err(|error| {
            failure(
                ContainerOpError::Handler(error),
                &prepared.provider,
                self.redactor,
            )
        })?;
        deleted.id = prepared.encoded_id(&deleted.id, request, self.redactor)?;
        Ok(deleted)
    }

    // LiteLLM's aretrieve_container: the async flag on, the awaited handler, and the raw provider id back
    pub async fn aretrieve_container(
        &self,
        request: &ContainerOperationRequest<'_>,
    ) -> Result<ContainerObject, ContainerOpFailure> {
        if let Some(object) = mock_retrieve(request.mock_response)
            .map_err(|error| failure(error, caller_provider(request), self.redactor))?
        {
            return Ok(object);
        }
        let prepared = self.prepare(request, RETRIEVE_CALL_TYPE)?;
        let call = prepared.handler_call(request, self.logging);
        self.handler.aretrieve(&call).await.map_err(|error| {
            failure(
                ContainerOpError::Handler(error),
                caller_provider(request),
                self.redactor,
            )
        })
    }

    // LiteLLM's adelete_container, which keeps the id the provider sent
    pub async fn adelete_container(
        &self,
        request: &ContainerOperationRequest<'_>,
    ) -> Result<DeleteContainerResult, ContainerOpFailure> {
        if let Some(deleted) = mock_delete(request.mock_response)
            .map_err(|error| failure(error, caller_provider(request), self.redactor))?
        {
            return Ok(deleted);
        }
        let prepared = self.prepare(request, DELETE_CALL_TYPE)?;
        let call = prepared.handler_call(request, self.logging);
        self.handler.adelete(&call).await.map_err(|error| {
            failure(
                ContainerOpError::Handler(error),
                caller_provider(request),
                self.redactor,
            )
        })
    }
}

#[path = "container_resource_ops/tests/support.rs"]
mod test_support;

#[path = "container_resource_ops/tests/mock.rs"]
mod mock_tests;

#[path = "container_resource_ops/tests/provider.rs"]
mod provider_tests;

#[path = "container_resource_ops/tests/logging.rs"]
mod logging_tests;

#[path = "container_resource_ops/tests/handler_args.rs"]
mod handler_args_tests;

#[path = "container_resource_ops/tests/encode.rs"]
mod encode_tests;

#[path = "container_resource_ops/tests/asynchronous.rs"]
mod asynchronous_tests;

#[path = "container_resource_ops/tests/errors.rs"]
mod error_tests;

#[path = "container_resource_ops/tests/real_handler.rs"]
mod real_handler_tests;
