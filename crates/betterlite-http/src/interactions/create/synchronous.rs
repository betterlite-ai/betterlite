use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use reqwest::blocking::Client as BlockingClient;
use reqwest::blocking::Response as BlockingResponse;
use url::Url;

use super::{
    CreateInteractionConfig, CreateInteractionError, CreateInteractionRequest, CreateOutcome,
    PreparedCreate, finish, handle_failure, prepare,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers};
use crate::interactions::base::{Logging, MAX_RESPONSE_BYTES, RequestFailure, UpstreamResponse};
use crate::interactions::lifecycle::check_status;
use crate::interactions::streaming::{StreamError, create_sync_iterator};

pub fn create_interaction<C>(
    config: &C,
    clients: &mut HttpClients,
    logging: &mut Logging,
    supplied: Option<BlockingClient>,
    request: &CreateInteractionRequest<'_>,
) -> Result<CreateOutcome<C::Output, C::SyncStream>, CreateInteractionError<C::Error>>
where
    C: CreateInteractionConfig,
{
    let own_client = supplied.is_none();
    let client = clients
        .sync_client(request.litellm_params, supplied)
        .map_err(CreateInteractionError::Client)?;
    let prepared = prepare(config, logging, request)?;
    let proxy = own_client.then_some(&*clients.proxy);
    let deadline = request.timeout.unwrap_or(request.request_timeout);
    let sent = send_blocking(&client, proxy, &prepared, deadline)
        .map_err(|failure| handle_failure(config, failure))?;
    if !request.stream {
        let response = buffer_blocking(sent).map_err(|failure| handle_failure(config, failure))?;
        return finish(config, logging, request.model, &response).map(CreateOutcome::Response);
    }
    match create_sync_iterator(sent, request.model, logging, config) {
        Ok(stream) => Ok(CreateOutcome::Stream(stream)),
        Err(StreamError::Read(failure)) => Err(handle_failure(config, failure)),
        Err(StreamError::Provider(error)) => Err(CreateInteractionError::Provider(error)),
    }
}

fn send_blocking(
    client: &BlockingClient,
    proxy: Option<&ProxyPolicy>,
    prepared: &PreparedCreate,
    deadline: Duration,
) -> Result<BlockingResponse, RequestFailure> {
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
        .map_err(RequestFailure::Transport)
}

fn buffer_blocking(sent: BlockingResponse) -> Result<UpstreamResponse, RequestFailure> {
    let status_code = sent.status().as_u16();
    let headers = response_headers(sent.headers());
    let body = read_body(sent, MAX_RESPONSE_BYTES)?;
    check_status(status_code, headers, body)
}
