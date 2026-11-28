pub mod asynchronous;
pub mod error;
pub mod execution;
mod mock;
mod prepare;

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::ContainerFileListResponse;
use betterlite_openai::containers::container_file_http::PreCallHook;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::containers::container_file_http::transport::{
    AsyncClient, BlockingClient, ClientSource,
};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::error::{ContainerFileListError, ContainerFileListFailure, failure};
use self::execution::blocking::container_file_list_handler;
use self::prepare::{Prepared, prepare_call};
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;

pub(crate) const DEFAULT_PROVIDER: &str = "openai";
// LiteLLM's declared default for this function's timeout argument, before `timeout or DEFAULT_REQUEST_TIMEOUT`
pub(crate) const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;
pub(crate) const CALL_TYPE: &str = "list_container_files";

// the named arguments list_container_files takes, with the values @client injects split out
pub struct ContainerFileListRequest<'request> {
    pub container_id: &'request str,
    pub after: Option<&'request str>,
    pub limit: Option<i64>,
    pub order: Option<&'request str>,
    // the declared default is 600 seconds; an explicit zero is LiteLLM's `timeout or DEFAULT_REQUEST_TIMEOUT`
    pub timeout: Option<f64>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<&'request str>,
    pub api_version: Option<&'request str>,
    // the declared default is "openai"
    pub custom_llm_provider: Option<&'request str>,
    pub extra_headers: Option<&'request BTreeMap<String, String>>,
    pub extra_query: Option<&'request IndexMap<String, ParamValue>>,
    // accepted to match the signature; LiteLLM never forwards it
    pub extra_body: Option<&'request IndexMap<String, JsonValue>>,
    pub kwargs: &'request IndexMap<String, Json>,
}

// the caller's named arguments after their declared defaults: the openai provider and 600 seconds
#[derive(Clone, Copy)]
pub(crate) struct Caller<'caller> {
    pub(crate) provider: &'caller str,
    pub(crate) timeout_seconds: f64,
}

pub(crate) fn caller_defaults<'caller>(
    request: &ContainerFileListRequest<'caller>,
) -> Caller<'caller> {
    Caller {
        provider: request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER),
        timeout_seconds: request.timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
    }
}

// the collaborators the operation needs: B247's logging object, B185's redactor and REQUEST_TIMEOUT's seconds
#[derive(Clone, Copy)]
pub struct ContainerFileListDeps<'deps> {
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    // LiteLLM's request_timeout constant that REQUEST_TIMEOUT overrides, used only where LiteLLM reaches it
    pub request_timeout_seconds: f64,
}

// LiteLLM's list_container_files: mock first, then the blocking handler runs the call it prepared
pub fn list_container_files<Source, Hook, Configs>(
    clients: &Source,
    hook: &Hook,
    configs: &Configs,
    deps: ContainerFileListDeps<'_>,
    request: &ContainerFileListRequest<'_>,
) -> Result<ContainerFileListResponse, ContainerFileListFailure>
where
    Source: ClientSource<Blocking: BlockingClient, Asynchronous: AsyncClient>,
    Hook: PreCallHook,
    Configs: ContainerConfigLookup,
{
    let prepared = prepare_call(configs, &deps, request)?;
    let ready = match prepared {
        Prepared::Mock(response) => return Ok(response),
        Prepared::Call(ready) => ready,
    };
    let call = ready.handler_call::<Source>(request, deps.logging);
    container_file_list_handler(clients, hook, &call).map_err(|error| {
        failure(
            ContainerFileListError::Handler(error),
            &ready.provider,
            deps.redactor,
        )
    })
}

#[path = "container_file_list/tests/support.rs"]
mod test_support;

#[path = "container_file_list/tests/operation_calls.rs"]
mod operation_calls;

#[path = "container_file_list/tests/handler_calls.rs"]
mod handler_calls;

#[path = "container_file_list/tests/failure_calls.rs"]
mod failure_calls;
