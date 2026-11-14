use betterlite_config::proxy::ProxyPolicy;
use reqwest::Client;

use super::read_call::{PreparedRead, ReadCall, ReadOperation};
use super::{AgentRequestError, AgentsApiConfig};
use crate::interactions::base::transport::response::response_headers;
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers, request_url};
use crate::interactions::base::{
    MAX_RESPONSE_BYTES, RequestFailure, UpstreamResponse, handle_error,
};
use crate::proxy::limits::read_bounded;

pub(crate) async fn asynchronous<Config, Operation>(
    mut call: ReadCall<'_, Config, Operation>,
    client: &Client,
    proxy: Option<&ProxyPolicy>,
) -> Result<Operation::Output, AgentRequestError<Config::Error>>
where
    Config: AgentsApiConfig,
    Operation: ReadOperation<Config>,
{
    let prepared = call.prepared()?;
    let response = send_async(client, proxy, &prepared)
        .await
        .map_err(|failure| handle_error(call.config, failure))?;
    call.finish(&response)
}

async fn send_async(
    client: &Client,
    proxy: Option<&ProxyPolicy>,
    prepared: &PreparedRead,
) -> Result<UpstreamResponse, RequestFailure> {
    let url = request_url(&prepared.request)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    let mut sent = client
        .get(url)
        .headers(request_headers(&prepared.headers)?)
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
