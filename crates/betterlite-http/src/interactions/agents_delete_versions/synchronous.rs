use betterlite_config::proxy::ProxyPolicy;
use reqwest::blocking::Client as BlockingClient;
use reqwest::blocking::Response as BlockingResponse;

use super::{
    AgentRequest, AgentsConfig, AgentsDeleteVersionsError, DeleteCall, VersionCall, ensure_success,
    finish_delete, finish_versions, handle_failure, prepare_delete, prepare_versions,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers, request_url};
use crate::interactions::base::{Logging, MAX_RESPONSE_BYTES, RequestFailure, UpstreamResponse};

pub fn delete_agent<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut Logging,
    supplied: Option<BlockingClient>,
    request: &AgentRequest<'_>,
) -> Result<C::DeleteOutput, AgentsDeleteVersionsError<C::Error>>
where
    C: AgentsConfig,
{
    let own_client = supplied.is_none();
    let client = clients
        .sync_client(request.litellm_params, supplied)
        .map_err(AgentsDeleteVersionsError::Client)?;
    let call = prepare_delete(config, logging, request)?;
    let proxy = own_client.then_some(&*clients.proxy);
    let response = send_delete(&client, proxy, &call)
        .and_then(finish_blocking)
        .and_then(ensure_success)
        .map_err(|failure| handle_failure(config, failure))?;
    finish_delete(config, logging, &response, request.name)
}

pub fn list_agent_versions<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut Logging,
    supplied: Option<BlockingClient>,
    request: &AgentRequest<'_>,
) -> Result<C::VersionsOutput, AgentsDeleteVersionsError<C::Error>>
where
    C: AgentsConfig,
{
    let own_client = supplied.is_none();
    let client = clients
        .sync_client(request.litellm_params, supplied)
        .map_err(AgentsDeleteVersionsError::Client)?;
    let call = prepare_versions(config, logging, request)?;
    let proxy = own_client.then_some(&*clients.proxy);
    let response = send_versions(&client, proxy, &call)
        .and_then(finish_blocking)
        .and_then(ensure_success)
        .map_err(|failure| handle_failure(config, failure))?;
    finish_versions(config, logging, &response, request.name)
}

fn send_delete(
    client: &BlockingClient,
    proxy: Option<&ProxyPolicy>,
    call: &DeleteCall,
) -> Result<BlockingResponse, RequestFailure> {
    let url = request_url(&call.target)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    client
        .delete(url)
        .headers(request_headers(&call.headers)?)
        .timeout(call.timeout)
        .send()
        .map_err(RequestFailure::Transport)
}

// LiteLLM's GET for the versions list passes no deadline, so the client's own applies
fn send_versions(
    client: &BlockingClient,
    proxy: Option<&ProxyPolicy>,
    call: &VersionCall,
) -> Result<BlockingResponse, RequestFailure> {
    let url = request_url(&call.prepared)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    client
        .get(url)
        .headers(request_headers(&call.headers)?)
        .send()
        .map_err(RequestFailure::Transport)
}

fn finish_blocking(sent: BlockingResponse) -> Result<UpstreamResponse, RequestFailure> {
    let status_code = sent.status().as_u16();
    let headers = response_headers(sent.headers());
    let body = read_body(sent, MAX_RESPONSE_BYTES)?;
    Ok(UpstreamResponse {
        status_code,
        headers,
        body,
    })
}
