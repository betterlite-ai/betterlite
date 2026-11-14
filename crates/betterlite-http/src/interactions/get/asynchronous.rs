use betterlite_config::proxy::ProxyPolicy;
use reqwest::Client;

use super::{
    GetInteractionConfig, GetInteractionError, GetInteractionRequest, finish, handle_failure,
    prepare,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::clients::asynchronous::async_client_for;
use crate::interactions::base::transport::response::response_headers;
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers, request_url};
use crate::interactions::base::{
    Logging, MAX_RESPONSE_BYTES, PreparedRequest, RequestFailure, RequestHeaders, UpstreamResponse,
};
use crate::proxy::limits::read_bounded;

pub async fn async_get_interaction<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut Logging,
    supplied: Option<Client>,
    request: &GetInteractionRequest<'_>,
) -> Result<C::Output, GetInteractionError<C::Error>>
where
    C: GetInteractionConfig,
{
    let own_client = supplied.is_none();
    let client = async_client_for(clients, request.provider, request.litellm_params, supplied)
        .map_err(GetInteractionError::Client)?;
    let (headers, prepared) = prepare(config, logging, request)?;
    let proxy = own_client.then_some(&*clients.proxy);
    let response = send_async(&client, proxy, &prepared, &headers)
        .await
        .map_err(|failure| handle_failure(config, failure))?;
    finish(config, logging, &response)
}

async fn send_async(
    client: &Client,
    proxy: Option<&ProxyPolicy>,
    prepared: &PreparedRequest,
    headers: &RequestHeaders,
) -> Result<UpstreamResponse, RequestFailure> {
    let url = request_url(prepared)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    let mut sent = client
        .get(url)
        .headers(request_headers(headers)?)
        .send()
        .await
        .map_err(RequestFailure::Transport)?;
    let status_code = sent.status().as_u16();
    let received = response_headers(sent.headers());
    let body = read_bounded(&mut sent, MAX_RESPONSE_BYTES)
        .await
        .map_err(RequestFailure::from)?;
    Ok(UpstreamResponse {
        status_code,
        headers: received,
        body,
    })
}
