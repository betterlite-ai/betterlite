pub mod error;
pub mod handler;
mod kind;
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
use self::kind::{Delete, Kind, Retrieve};
use self::prepare::{DEFAULT_PROVIDER, Prepared};
use crate::api::container_file_endpoints::handler::{ContainerConfigLookup, ContainerIdDecoder};

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

// the mock answer, or the prepared call the handler still has to make
enum Begin<'deps, Reply> {
    Mock(Reply),
    Call(Prepared<'deps>),
}

// LiteLLM spells the blocking and async form of each operation out twice; one body pair serves both
macro_rules! operations {
    ($($sync:ident, $asynchronous:ident, $kind:ident, $reply:ty;)+) => { $(
        // LiteLLM's operation: the mock short circuit, then the blocking handler and the id re-encoding
        pub fn $sync(
            &self,
            request: &ContainerOperationRequest<'_>,
        ) -> Result<$reply, ContainerOpFailure> {
            let prepared = match self.begin::<$kind>(request)? {
                Begin::Mock(reply) => return Ok(reply),
                Begin::Call(prepared) => prepared,
            };
            let call = prepared.handler_call(request, self.logging);
            let mut reply = $kind::call(self.handler, &call).map_err(|error| {
                failure(
                    ContainerOpError::Handler(error),
                    &prepared.provider,
                    self.redactor,
                )
            })?;
            let id = prepared.encoded_id($kind::id(&reply), request, self.redactor)?;
            $kind::replace_id(&mut reply, id);
            Ok(reply)
        }

        // the async form awaits B3365's async handler and keeps the provider's own id instead of re-encoding
        pub async fn $asynchronous(
            &self,
            request: &ContainerOperationRequest<'_>,
        ) -> Result<$reply, ContainerOpFailure> {
            let prepared = match self.begin::<$kind>(request)? {
                Begin::Mock(reply) => return Ok(reply),
                Begin::Call(prepared) => prepared,
            };
            let call = prepared.handler_call(request, self.logging);
            $kind::call_async(self.handler, &call)
                .await
                .map_err(|error| {
                    failure(
                        ContainerOpError::Handler(error),
                        request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER),
                        self.redactor,
                    )
                })
        }
    )+ };
}

impl<'deps, Handler, Decoder, Configs> ContainerOperations<'deps, Handler, Decoder, Configs>
where
    Handler: ContainerHandler,
    Decoder: ContainerIdDecoder,
    Configs: ContainerConfigLookup,
{
    // LiteLLM checks the mock first: only a real call decodes the id, looks up the config and updates the logging
    fn begin<Operation: Kind>(
        &self,
        request: &ContainerOperationRequest<'_>,
    ) -> Result<Begin<'deps, Operation::Reply>, ContainerOpFailure> {
        if let Some(reply) = Operation::mock(request.mock_response).map_err(|error| {
            failure(
                error,
                request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER),
                self.redactor,
            )
        })? {
            return Ok(Begin::Mock(reply));
        }
        let prepared = self.prepare(request, Operation::CALL_TYPE)?;
        Ok(Begin::Call(prepared))
    }

    operations! {
        retrieve_container, aretrieve_container, Retrieve, ContainerObject;
        delete_container, adelete_container, Delete, DeleteContainerResult;
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
