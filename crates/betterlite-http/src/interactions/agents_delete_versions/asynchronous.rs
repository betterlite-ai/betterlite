use betterlite_config::proxy::ProxyPolicy;
use reqwest::Client;

use super::{
    AgentRequest, AgentsConfig, AgentsDeleteVersionsError, CallKind, PreparedCall, ensure_success,
    handle_failure, prepare, record_post_call,
};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::clients::async_client;
use crate::interactions::base::transport::response::response_headers;
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers, request_url};
use crate::interactions::base::{
    ClientError, Logging, MAX_RESPONSE_BYTES, RequestFailure, UpstreamResponse,
};
use crate::proxy::limits::read_bounded;

macro_rules! async_agent_entries {
    ($($method:ident: $kind:expr, $output:ident, $transform:ident);* $(;)?) => {
        $(
            pub async fn $method<C: AgentsConfig>(
                config: &C,
                clients: &mut HttpClients,
                logging: &mut Logging,
                supplied: Option<Client>,
                request: &AgentRequest<'_>,
            ) -> Result<C::$output, AgentsDeleteVersionsError<C::Error>> {
                let (client, proxy) = open_client(clients, supplied, request)
                    .map_err(AgentsDeleteVersionsError::Client)?;
                let call = prepare(config, logging, request, $kind)?;
                let response = send(&client, proxy, &call)
                    .await
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

async_agent_entries!(
    async_delete_agent: CallKind::Delete, DeleteOutput, transform_delete_response;
    async_list_agent_versions: CallKind::Versions, VersionsOutput, transform_versions_response;
);

fn open_client<'clients>(
    clients: &'clients mut HttpClients,
    supplied: Option<Client>,
    request: &AgentRequest<'_>,
) -> Result<(Client, Option<&'clients ProxyPolicy>), ClientError> {
    let own_client = supplied.is_none();
    let client = async_client(clients, request.litellm_params, supplied)?;
    Ok((client, own_client.then_some(&*clients.proxy)))
}

async fn send(
    client: &Client,
    proxy: Option<&ProxyPolicy>,
    call: &PreparedCall,
) -> Result<UpstreamResponse, RequestFailure> {
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
    let mut sent = request.send().await.map_err(RequestFailure::Transport)?;
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
