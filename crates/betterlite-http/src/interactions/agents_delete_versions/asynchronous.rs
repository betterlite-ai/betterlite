use betterlite_config::proxy::ProxyPolicy;
use reqwest::Client;

use super::{
    AgentRequest, AgentsConfig, AgentsDeleteVersionsError, DeleteCall, VersionCall, ensure_success,
    finish_delete, finish_versions, handle_failure, prepare_delete, prepare_versions,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::clients::async_client;
use crate::interactions::base::transport::response::response_headers;
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers, request_url};
use crate::interactions::base::{Logging, MAX_RESPONSE_BYTES, RequestFailure, UpstreamResponse};
use crate::proxy::limits::read_bounded;

pub async fn async_delete_agent<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut Logging,
    supplied: Option<Client>,
    request: &AgentRequest<'_>,
) -> Result<C::DeleteOutput, AgentsDeleteVersionsError<C::Error>>
where
    C: AgentsConfig,
{
    let own_client = supplied.is_none();
    let client = async_client(clients, request.litellm_params, supplied)
        .map_err(AgentsDeleteVersionsError::Client)?;
    let call = prepare_delete(config, logging, request)?;
    let proxy = own_client.then_some(&*clients.proxy);
    let response = send_delete(&client, proxy, &call)
        .await
        .and_then(ensure_success)
        .map_err(|failure| handle_failure(config, failure))?;
    finish_delete(config, logging, &response, request.name)
}

pub async fn async_list_agent_versions<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut Logging,
    supplied: Option<Client>,
    request: &AgentRequest<'_>,
) -> Result<C::VersionsOutput, AgentsDeleteVersionsError<C::Error>>
where
    C: AgentsConfig,
{
    let own_client = supplied.is_none();
    let client = async_client(clients, request.litellm_params, supplied)
        .map_err(AgentsDeleteVersionsError::Client)?;
    let call = prepare_versions(config, logging, request)?;
    let proxy = own_client.then_some(&*clients.proxy);
    let response = send_versions(&client, proxy, &call)
        .await
        .and_then(ensure_success)
        .map_err(|failure| handle_failure(config, failure))?;
    finish_versions(config, logging, &response, request.name)
}

async fn send_delete(
    client: &Client,
    proxy: Option<&ProxyPolicy>,
    call: &DeleteCall,
) -> Result<UpstreamResponse, RequestFailure> {
    let url = request_url(&call.target)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    let mut sent = client
        .delete(url)
        .headers(request_headers(&call.headers)?)
        .timeout(call.timeout)
        .send()
        .await
        .map_err(RequestFailure::Transport)?;
    let status_code = sent.status().as_u16();
    let headers = response_headers(sent.headers());
    let body = read_bounded(&mut sent, MAX_RESPONSE_BYTES)
        .await
        .map_err(RequestFailure::from)?;
    Ok(UpstreamResponse {
        status_code,
        headers,
        body,
    })
}

// LiteLLM's GET for the versions list passes no deadline, so the client's own applies
async fn send_versions(
    client: &Client,
    proxy: Option<&ProxyPolicy>,
    call: &VersionCall,
) -> Result<UpstreamResponse, RequestFailure> {
    let url = request_url(&call.prepared)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    let mut sent = client
        .get(url)
        .headers(request_headers(&call.headers)?)
        .send()
        .await
        .map_err(RequestFailure::Transport)?;
    let status_code = sent.status().as_u16();
    let headers = response_headers(sent.headers());
    let body = read_bounded(&mut sent, MAX_RESPONSE_BYTES)
        .await
        .map_err(RequestFailure::from)?;
    Ok(UpstreamResponse {
        status_code,
        headers,
        body,
    })
}
