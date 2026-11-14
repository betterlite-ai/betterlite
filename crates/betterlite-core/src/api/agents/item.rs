pub mod call;
pub mod error;

mod logging;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_http::interactions::agents_create_read::get_async::async_get_agent;
use betterlite_http::interactions::agents_create_read::get_sync::get_agent;
use betterlite_http::interactions::agents_create_read::{AgentsApiConfig, GetAgentRequest};
use betterlite_http::interactions::agents_delete_versions::AgentsConfig;
use betterlite_http::interactions::agents_delete_versions::asynchronous::{
    async_delete_agent, async_list_agent_versions,
};
use betterlite_http::interactions::agents_delete_versions::synchronous::{
    delete_agent, list_agent_versions,
};
use betterlite_http::interactions::base::Logging;
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use self::call::{Operation, named_request, prepare};
use self::error::{AgentItemFailureError, ProviderFailure, outcome};
use self::logging::AgentHttpLogging;

// the caller's side of one named-agent operation, as LiteLLM's get, delete and list_versions take it
pub struct AgentItemRequest<'request> {
    pub name: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub extra_headers: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    // B198's REQUEST_TIMEOUT; only the delete call falls back to it
    pub request_timeout: RequestTimeout,
    pub kwargs: &'request IndexMap<String, Json>,
}

// the collaborators one operation writes into: clients, the typed logging state, the pre/post record B3502 fills, and the redactor exception_type scrubs the message with
pub struct AgentItemDeps<'deps> {
    pub clients: &'deps mut HttpClients,
    pub logging: &'deps mut LoggingState,
    pub http: &'deps mut Logging,
    pub redactor: &'deps Redactor,
}

macro_rules! agent_entries {
    (read $method:ident, $send:path) => {
        pub fn $method<Config>(
            config: &Config,
            deps: AgentItemDeps<'_>,
            request: &AgentItemRequest<'_>,
        ) -> Result<Config::GetOutput, AgentItemFailureError<Config::Error>>
        where
            Config: AgentsApiConfig,
            Config::Error: ProviderFailure,
        {
            let AgentItemDeps {
                clients,
                logging,
                http,
                redactor,
            } = deps;
            let prepared = prepare(request, Operation::Get, logging, redactor)?;
            let sent = GetAgentRequest {
                name: request.name,
                litellm_params: &prepared.params,
                extra_headers: request.extra_headers.clone(),
                timeout: request.timeout,
            };
            let mut recording = AgentHttpLogging(http);
            let failed = $send(config, clients, &mut recording, None, &sent);
            outcome(failed, request.name, &prepared.provider, redactor)
        }
    };
    (aread $method:ident, $send:path) => {
        pub async fn $method<Config>(
            config: &Config,
            deps: AgentItemDeps<'_>,
            request: &AgentItemRequest<'_>,
        ) -> Result<Config::GetOutput, AgentItemFailureError<Config::Error>>
        where
            Config: AgentsApiConfig,
            Config::Error: ProviderFailure,
        {
            let AgentItemDeps {
                clients,
                logging,
                http,
                redactor,
            } = deps;
            let prepared = prepare(request, Operation::Get, logging, redactor)?;
            let sent = GetAgentRequest {
                name: request.name,
                litellm_params: &prepared.params,
                extra_headers: request.extra_headers.clone(),
                timeout: request.timeout,
            };
            let mut recording = AgentHttpLogging(http);
            let failed = $send(config, clients, &mut recording, None, &sent).await;
            outcome(failed, request.name, &prepared.provider, redactor)
        }
    };
    (named $method:ident, $operation:ident, $output:ident, $send:path) => {
        pub fn $method<Config>(
            config: &Config,
            deps: AgentItemDeps<'_>,
            request: &AgentItemRequest<'_>,
        ) -> Result<Config::$output, AgentItemFailureError<Config::Error>>
        where
            Config: AgentsConfig,
            Config::Error: ProviderFailure,
        {
            let AgentItemDeps {
                clients,
                logging,
                http,
                redactor,
            } = deps;
            let prepared = prepare(request, Operation::$operation, logging, redactor)?;
            let sent = named_request(request, &prepared.params);
            let failed = $send(config, clients, http, None, &sent);
            outcome(failed, request.name, &prepared.provider, redactor)
        }
    };
    (anamed $method:ident, $operation:ident, $output:ident, $send:path) => {
        pub async fn $method<Config>(
            config: &Config,
            deps: AgentItemDeps<'_>,
            request: &AgentItemRequest<'_>,
        ) -> Result<Config::$output, AgentItemFailureError<Config::Error>>
        where
            Config: AgentsConfig,
            Config::Error: ProviderFailure,
        {
            let AgentItemDeps {
                clients,
                logging,
                http,
                redactor,
            } = deps;
            let prepared = prepare(request, Operation::$operation, logging, redactor)?;
            let sent = named_request(request, &prepared.params);
            let failed = $send(config, clients, http, None, &sent).await;
            outcome(failed, request.name, &prepared.provider, redactor)
        }
    };
}

agent_entries!(read get, get_agent);
agent_entries!(aread aget, async_get_agent);
agent_entries!(named delete, Delete, DeleteOutput, delete_agent);
agent_entries!(anamed adelete, Delete, DeleteOutput, async_delete_agent);
agent_entries!(named list_versions, Versions, VersionsOutput, list_agent_versions);
agent_entries!(anamed alist_versions, Versions, VersionsOutput, async_list_agent_versions);

#[path = "item/tests/agent_item_calls.rs"]
mod agent_item_calls;
