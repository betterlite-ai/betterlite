use reqwest::blocking::Client as BlockingClient;

use super::client_call::blocking_client;
use super::list_call::ListOperation;
use super::logging::AgentLogging;
use super::read_call::ReadCall;
use super::{AgentRequestError, AgentsApiConfig, ListAgentsRequest};
use crate::interactions::base::clients::HttpClients;

pub fn list_agents<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut dyn AgentLogging,
    supplied: Option<BlockingClient>,
    request: &ListAgentsRequest<'_>,
) -> Result<C::ListOutput, AgentRequestError<C::Error>>
where
    C: AgentsApiConfig,
{
    let (client, proxy) = blocking_client(clients, request.litellm_params, supplied)
        .map_err(AgentRequestError::Client)?;
    ReadCall {
        config,
        logging,
        litellm_params: request.litellm_params,
        extra_headers: &request.extra_headers,
        input: "list_agents",
        operation: ListOperation,
    }
    .blocking(&client, proxy)
}
