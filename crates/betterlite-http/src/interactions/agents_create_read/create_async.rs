use reqwest::Client;

use super::async_client_call::async_client;
use super::create_call::CreateCall;
use super::create_call_async::asynchronous;
use super::logging::AgentLogging;
use super::{AgentRequestError, AgentsApiConfig, CreateAgentRequest};
use crate::interactions::base::clients::HttpClients;

pub async fn async_create_agent<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut dyn AgentLogging,
    supplied: Option<Client>,
    request: &CreateAgentRequest<'_>,
) -> Result<C::CreateOutput, AgentRequestError<C::Error>>
where
    C: AgentsApiConfig,
{
    let (client, proxy) = async_client(clients, request.litellm_params, supplied)
        .map_err(AgentRequestError::Client)?;
    asynchronous(
        CreateCall {
            config,
            logging,
            request,
        },
        &client,
        proxy,
    )
    .await
}
