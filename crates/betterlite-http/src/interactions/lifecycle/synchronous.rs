use std::sync::Arc;
use std::time::Duration;

use reqwest::blocking::Client as BlockingClient;
use reqwest::blocking::Response as BlockingResponse;
use url::Url;

use super::{
    CancelResult, DeleteResult, Fault, InteractionLifecycleConfig, InteractionsHttpHandler,
    LifecycleError, LifecycleRequest as Request, Operation, Outbound, check_status, handle_failure,
    prepare,
};
use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers};
use crate::interactions::base::{
    MAX_RESPONSE_BYTES, PreparedRequest, RequestFailure, RequestHeaders, UpstreamResponse,
};

impl<C> InteractionsHttpHandler<'_, C, BlockingClient>
where
    C: InteractionLifecycleConfig,
{
    pub fn delete_interaction(&mut self, request: &Request<'_>) -> DeleteResult<C> {
        let outbound = self.connect(request)?;
        let response = self.exchange(&outbound, request, Operation::Delete)?;
        self.config
            .transform_delete_response(&response, &mut self.logging, request.interaction_id)
            .map_err(LifecycleError::Provider)
    }

    pub fn cancel_interaction(&mut self, request: &Request<'_>) -> CancelResult<C> {
        let outbound = self.connect(request)?;
        let response = self.exchange(&outbound, request, Operation::Cancel)?;
        self.config
            .transform_cancel_response(&response, &mut self.logging)
            .map_err(LifecycleError::Provider)
    }

    fn connect(&mut self, request: &Request<'_>) -> Result<Outbound<BlockingClient>, Fault<C>> {
        let own_client = self.supplied.is_none();
        let client = self
            .clients
            .sync_client(request.litellm_params, self.supplied.clone())
            .map_err(LifecycleError::Client)?;
        let proxy = own_client.then(|| Arc::clone(&self.clients.proxy));
        Ok(Outbound { client, proxy })
    }

    fn exchange(
        &mut self,
        outbound: &Outbound<BlockingClient>,
        request: &Request<'_>,
        operation: Operation,
    ) -> Result<UpstreamResponse, Fault<C>> {
        let (headers, prepared) = prepare(self.config, &mut self.logging, request, operation)?;
        send_blocking(
            outbound,
            &prepared,
            &headers,
            operation,
            request.timeout.unwrap_or(request.request_timeout),
        )
        .and_then(finish_blocking)
        .map_err(|failure| handle_failure(self.config, failure))
    }
}

// LiteLLM never sends the data a delete transform returns; only a cancel posts its body as JSON
fn send_blocking(
    outbound: &Outbound<BlockingClient>,
    prepared: &PreparedRequest,
    headers: &RequestHeaders,
    operation: Operation,
    deadline: Duration,
) -> Result<BlockingResponse, RequestFailure> {
    let url = Url::parse(&prepared.url).map_err(RequestFailure::Url)?;
    if let Some(policy) = outbound.proxy.as_deref() {
        refuse_unusable_proxy(policy, &url)?;
    }
    let request = match operation {
        Operation::Delete => outbound.client.delete(url),
        Operation::Cancel => outbound.client.post(url).json(&prepared.params),
    };
    request
        .headers(request_headers(headers)?)
        .timeout(deadline)
        .send()
        .map_err(RequestFailure::Transport)
}

fn finish_blocking(sent: BlockingResponse) -> Result<UpstreamResponse, RequestFailure> {
    let status_code = sent.status().as_u16();
    let received = response_headers(sent.headers());
    let body = read_body(sent, MAX_RESPONSE_BYTES)?;
    check_status(status_code, received, body)
}
