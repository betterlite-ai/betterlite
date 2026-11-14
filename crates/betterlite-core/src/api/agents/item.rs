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

pub fn get<Config>(
    config: &Config,
    deps: AgentItemDeps<'_>,
    request: &AgentItemRequest<'_>,
) -> Result<Config::GetOutput, AgentItemFailureError<Config::Error>>
where
    Config: AgentsApiConfig,
    Config::Error: ProviderFailure,
{
    let prepared = prepare(request, Operation::Get, deps.logging, deps.redactor)?;
    let sent = GetAgentRequest {
        name: request.name,
        litellm_params: &prepared.params,
        extra_headers: request.extra_headers.clone(),
        timeout: request.timeout,
    };
    let mut recording = AgentHttpLogging(deps.http);
    let failed = get_agent(config, deps.clients, &mut recording, None, &sent);
    outcome(failed, request.name, &prepared.provider, deps.redactor)
}

pub async fn aget<Config>(
    config: &Config,
    deps: AgentItemDeps<'_>,
    request: &AgentItemRequest<'_>,
) -> Result<Config::GetOutput, AgentItemFailureError<Config::Error>>
where
    Config: AgentsApiConfig,
    Config::Error: ProviderFailure,
{
    let prepared = prepare(request, Operation::Get, deps.logging, deps.redactor)?;
    let sent = GetAgentRequest {
        name: request.name,
        litellm_params: &prepared.params,
        extra_headers: request.extra_headers.clone(),
        timeout: request.timeout,
    };
    let mut recording = AgentHttpLogging(deps.http);
    let failed = async_get_agent(config, deps.clients, &mut recording, None, &sent).await;
    outcome(failed, request.name, &prepared.provider, deps.redactor)
}

pub fn delete<Config>(
    config: &Config,
    deps: AgentItemDeps<'_>,
    request: &AgentItemRequest<'_>,
) -> Result<Config::DeleteOutput, AgentItemFailureError<Config::Error>>
where
    Config: AgentsConfig,
    Config::Error: ProviderFailure,
{
    let prepared = prepare(request, Operation::Delete, deps.logging, deps.redactor)?;
    let sent = named_request(request, &prepared.params);
    let failed = delete_agent(config, deps.clients, deps.http, None, &sent);
    outcome(failed, request.name, &prepared.provider, deps.redactor)
}

pub async fn adelete<Config>(
    config: &Config,
    deps: AgentItemDeps<'_>,
    request: &AgentItemRequest<'_>,
) -> Result<Config::DeleteOutput, AgentItemFailureError<Config::Error>>
where
    Config: AgentsConfig,
    Config::Error: ProviderFailure,
{
    let prepared = prepare(request, Operation::Delete, deps.logging, deps.redactor)?;
    let sent = named_request(request, &prepared.params);
    let failed = async_delete_agent(config, deps.clients, deps.http, None, &sent).await;
    outcome(failed, request.name, &prepared.provider, deps.redactor)
}

pub fn list_versions<Config>(
    config: &Config,
    deps: AgentItemDeps<'_>,
    request: &AgentItemRequest<'_>,
) -> Result<Config::VersionsOutput, AgentItemFailureError<Config::Error>>
where
    Config: AgentsConfig,
    Config::Error: ProviderFailure,
{
    let prepared = prepare(request, Operation::Versions, deps.logging, deps.redactor)?;
    let sent = named_request(request, &prepared.params);
    let failed = list_agent_versions(config, deps.clients, deps.http, None, &sent);
    outcome(failed, request.name, &prepared.provider, deps.redactor)
}

pub async fn alist_versions<Config>(
    config: &Config,
    deps: AgentItemDeps<'_>,
    request: &AgentItemRequest<'_>,
) -> Result<Config::VersionsOutput, AgentItemFailureError<Config::Error>>
where
    Config: AgentsConfig,
    Config::Error: ProviderFailure,
{
    let prepared = prepare(request, Operation::Versions, deps.logging, deps.redactor)?;
    let sent = named_request(request, &prepared.params);
    let failed = async_list_agent_versions(config, deps.clients, deps.http, None, &sent).await;
    outcome(failed, request.name, &prepared.provider, deps.redactor)
}

#[cfg(test)]
#[path = "item/tests/agent_item_calls.rs"]
mod agent_item_calls;
