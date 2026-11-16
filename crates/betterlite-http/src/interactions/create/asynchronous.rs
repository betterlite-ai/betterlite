use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use reqwest::Client;
use reqwest::Response;
use url::Url;

use super::{
    CreateInteractionConfig, CreateInteractionError, CreateInteractionRequest, CreateOutcome,
    PreparedCreate, finish, handle_failure, prepare,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::clients::asynchronous::async_client_for;
use crate::interactions::base::transport::response::response_headers;
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers};
use crate::interactions::base::{Logging, MAX_RESPONSE_BYTES, RequestFailure, UpstreamResponse};
use crate::interactions::lifecycle::check_status;
use crate::interactions::streaming::create_async_iterator;
use crate::proxy::limits::read_bounded;

pub async fn async_create_interaction<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut Logging,
    supplied: Option<Client>,
    request: &CreateInteractionRequest<'_>,
) -> Result<CreateOutcome<C::Output, C::AsyncStream>, CreateInteractionError<C::Error>>
where
    C: CreateInteractionConfig,
{
    let own_client = supplied.is_none();
    let client = async_client_for(clients, request.provider, request.litellm_params, supplied)
        .map_err(CreateInteractionError::Client)?;
    let prepared = prepare(config, logging, request)?;
    let proxy = own_client.then_some(&*clients.proxy);
    let deadline = request.timeout.unwrap_or(request.request_timeout);
    let sent = send_async(&client, proxy, &prepared, deadline)
        .await
        .map_err(|failure| handle_failure(config, failure))?;
    if !request.stream {
        let response = buffer_async(sent)
            .await
            .map_err(|failure| handle_failure(config, failure))?;
        return finish(config, logging, request.model, &response).map(CreateOutcome::Response);
    }
    create_async_iterator(&client, sent, request.model, logging, config)
        .await
        .map(CreateOutcome::Stream)
}

async fn send_async(
    client: &Client,
    proxy: Option<&ProxyPolicy>,
    prepared: &PreparedCreate,
    deadline: Duration,
) -> Result<Response, RequestFailure> {
    let url = Url::parse(&prepared.url).map_err(RequestFailure::Url)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    client
        .post(url)
        .json(&prepared.data)
        .headers(request_headers(&prepared.headers)?)
        .timeout(deadline)
        .send()
        .await
        .map_err(RequestFailure::Transport)
}

async fn buffer_async(mut sent: Response) -> Result<UpstreamResponse, RequestFailure> {
    let status_code = sent.status().as_u16();
    let headers = response_headers(sent.headers());
    let body = read_bounded(&mut sent, MAX_RESPONSE_BYTES)
        .await
        .map_err(RequestFailure::from)?;
    check_status(status_code, headers, body)
}
