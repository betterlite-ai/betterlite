use betterlite_config::proxy::ProxyPolicy;
use reqwest::blocking::Client as BlockingClient;
use reqwest::blocking::Response as BlockingResponse;

use super::{
    GetInteractionConfig, GetInteractionError, GetInteractionRequest, finish, handle_failure,
    prepare,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers, request_url};
use crate::interactions::base::{
    Logging, MAX_RESPONSE_BYTES, PreparedRequest, RequestFailure, RequestHeaders, UpstreamResponse,
};

pub fn get_interaction<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut Logging,
    supplied: Option<BlockingClient>,
    request: &GetInteractionRequest<'_>,
) -> Result<C::Output, GetInteractionError<C::Error>>
where
    C: GetInteractionConfig,
{
    let own_client = supplied.is_none();
    let client = clients
        .sync_client(request.litellm_params, supplied)
        .map_err(GetInteractionError::Client)?;
    let (headers, prepared) = prepare(config, logging, request)?;
    let proxy = own_client.then_some(&*clients.proxy);
    let sent = send_blocking(&client, proxy, &prepared, &headers)
        .and_then(finish_blocking)
        .map_err(|failure| handle_failure(config, failure))?;
    finish(config, logging, &sent)
}

fn send_blocking(
    client: &BlockingClient,
    proxy: Option<&ProxyPolicy>,
    prepared: &PreparedRequest,
    headers: &RequestHeaders,
) -> Result<BlockingResponse, RequestFailure> {
    let url = request_url(prepared)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    client
        .get(url)
        .headers(request_headers(headers)?)
        .send()
        .map_err(RequestFailure::Transport)
}

fn finish_blocking(sent: BlockingResponse) -> Result<UpstreamResponse, RequestFailure> {
    let status_code = sent.status().as_u16();
    let received = response_headers(sent.headers());
    let body = read_body(sent, MAX_RESPONSE_BYTES)?;
    Ok(UpstreamResponse {
        status_code,
        headers: received,
        body,
    })
}
