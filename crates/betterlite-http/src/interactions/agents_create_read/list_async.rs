use reqwest::Client;

use super::async_client_call::async_client;
use super::list_call::ListOperation;
use super::logging::AgentLogging;
use super::read_call::ReadCall;
use super::{AgentRequestError, AgentsApiConfig, ListAgentsRequest};
use crate::interactions::base::clients::HttpClients;

pub async fn async_list_agents<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut dyn AgentLogging,
    supplied: Option<Client>,
    request: &ListAgentsRequest<'_>,
) -> Result<C::ListOutput, AgentRequestError<C::Error>>
where
    C: AgentsApiConfig,
{
    let (client, proxy) = async_client(clients, request.litellm_params, supplied)
        .map_err(AgentRequestError::Client)?;
    ReadCall {
        config,
        logging,
        litellm_params: request.litellm_params,
        extra_headers: &request.extra_headers,
        operation: ListOperation,
    }
    .asynchronous(&client, proxy)
    .await
}
