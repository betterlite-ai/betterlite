use reqwest::Client;

use super::async_client_call::async_client;
use super::get_call::GetOperation;
use super::logging::AgentLogging;
use super::read_call::ReadCall;
use super::{AgentRequestError, AgentsApiConfig, GetAgentRequest};
use crate::interactions::base::clients::HttpClients;

pub async fn async_get_agent<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut dyn AgentLogging,
    supplied: Option<Client>,
    request: &GetAgentRequest<'_>,
) -> Result<C::GetOutput, AgentRequestError<C::Error>>
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
        operation: GetOperation { name: request.name },
    }
    .asynchronous(&client, proxy)
    .await
}
