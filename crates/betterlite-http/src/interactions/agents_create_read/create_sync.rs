use reqwest::blocking::Client as BlockingClient;

use super::client_call::blocking_client;
use super::create_call::CreateCall;
use super::logging::AgentLogging;
use super::{AgentRequestError, AgentsApiConfig, CreateAgentRequest};
use crate::interactions::base::clients::HttpClients;

pub fn create_agent<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut dyn AgentLogging,
    supplied: Option<BlockingClient>,
    request: &CreateAgentRequest<'_>,
) -> Result<C::CreateOutput, AgentRequestError<C::Error>>
where
    C: AgentsApiConfig,
{
    let (client, proxy) = blocking_client(clients, request.litellm_params, supplied)
        .map_err(AgentRequestError::Client)?;
    CreateCall {
        config,
        logging,
        request,
    }
    .blocking(&client, proxy)
}
