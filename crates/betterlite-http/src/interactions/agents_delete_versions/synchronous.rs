use betterlite_config::proxy::ProxyPolicy;
use reqwest::blocking::Client as BlockingClient;
use reqwest::blocking::Response as BlockingResponse;

use super::{
    AgentRequest, AgentsConfig, AgentsDeleteVersionsError, CallKind, PreparedCall, ensure_success,
    handle_failure, prepare, record_post_call,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers, request_url};
use crate::interactions::base::{
    ClientError, Logging, MAX_RESPONSE_BYTES, RequestFailure, UpstreamResponse,
};

macro_rules! agent_entries {
    ($($method:ident: $kind:expr, $output:ident, $transform:ident);* $(;)?) => {
        $(
            pub fn $method<C: AgentsConfig>(
                config: &C,
                clients: &mut HttpClients,
                logging: &mut Logging,
                supplied: Option<BlockingClient>,
                request: &AgentRequest<'_>,
            ) -> Result<C::$output, AgentsDeleteVersionsError<C::Error>> {
                let (client, proxy) = open_client(clients, supplied, request)
                    .map_err(AgentsDeleteVersionsError::Client)?;
                let call = prepare(config, logging, request, $kind)?;
                let response = send(&client, proxy, &call)
                    .and_then(finish_blocking)
                    .and_then(ensure_success)
                    .map_err(|failure| handle_failure(config, failure))?;
                record_post_call(logging, &response);
                config
                    .$transform(&response, request.name)
                    .map_err(AgentsDeleteVersionsError::Provider)
            }
        )*
    };
}

agent_entries!(
    delete_agent: CallKind::Delete, DeleteOutput, transform_delete_response;
    list_agent_versions: CallKind::Versions, VersionsOutput, transform_versions_response;
);

fn open_client<'clients>(
    clients: &'clients mut HttpClients,
    supplied: Option<BlockingClient>,
    request: &AgentRequest<'_>,
) -> Result<(BlockingClient, Option<&'clients ProxyPolicy>), ClientError> {
    let own_client = supplied.is_none();
    let client = clients.sync_client(request.litellm_params, supplied)?;
    Ok((client, own_client.then_some(&*clients.proxy)))
}

fn send(
    client: &BlockingClient,
    proxy: Option<&ProxyPolicy>,
    call: &PreparedCall,
) -> Result<BlockingResponse, RequestFailure> {
    let url = request_url(&call.target)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    let mut request = match call.method {
        CallKind::Delete => client.delete(url),
        CallKind::Versions => client.get(url),
    };
    request = request.headers(request_headers(&call.headers)?);
    if let Some(timeout) = call.timeout {
        request = request.timeout(timeout);
    }
    request.send().map_err(RequestFailure::Transport)
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
