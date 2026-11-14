use betterlite_config::proxy::ProxyPolicy;
use reqwest::Client;
use url::Url;

use super::create_call::{CreateCall, PreparedCreate, success};
use super::{AgentRequestError, AgentsApiConfig};
use crate::interactions::base::transport::response::response_headers;
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers};
use crate::interactions::base::{
    MAX_RESPONSE_BYTES, RequestFailure, UpstreamResponse, handle_error,
};
use crate::proxy::limits::read_bounded;

impl<Config> CreateCall<'_, Config>
where
    Config: AgentsApiConfig,
{
    pub(crate) async fn asynchronous(
        mut self,
        client: &Client,
        proxy: Option<&ProxyPolicy>,
    ) -> Result<Config::CreateOutput, AgentRequestError<Config::Error>> {
        let prepared = self.prepared()?;
        let response = send_async(client, proxy, &prepared)
            .await
            .map_err(|failure| handle_error(self.config, failure))?;
        self.finish(&prepared, &response)
    }
}

async fn send_async(
    client: &Client,
    proxy: Option<&ProxyPolicy>,
    prepared: &PreparedCreate,
) -> Result<UpstreamResponse, RequestFailure> {
    let url = Url::parse(&prepared.url).map_err(RequestFailure::Url)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    let mut sent = client
        .post(url)
        .headers(request_headers(&prepared.headers)?)
        .json(&prepared.data)
        .timeout(prepared.timeout)
        .send()
        .await
        .map_err(RequestFailure::Transport)?;
    let status_code = sent.status().as_u16();
    let headers = response_headers(sent.headers());
    let body = read_bounded(&mut sent, MAX_RESPONSE_BYTES)
        .await
        .map_err(RequestFailure::from)?;
    success(status_code, headers, body)
}
