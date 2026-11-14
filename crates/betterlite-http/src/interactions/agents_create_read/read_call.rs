use std::collections::BTreeMap;

use betterlite_config::proxy::ProxyPolicy;
use reqwest::blocking::Client as BlockingClient;
use secrecy::SecretString;

use super::logging::{AgentLogging, AgentPostCall, AgentPreCall};
use super::{AgentRequestError, AgentsApiConfig, body_text};
use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers, request_url};
use crate::interactions::base::{
    MAX_RESPONSE_BYTES, Params, PreparedRequest, RequestFailure, RequestHeaders, UpstreamResponse,
    api_base, handle_error,
};

pub(crate) trait ReadOperation<Config>
where
    Config: AgentsApiConfig,
{
    type Output;

    fn input(&self) -> &str;

    fn transform_request(
        &self,
        config: &Config,
        api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, Config::Error>;

    fn transform_response(
        &self,
        config: &Config,
        response: &UpstreamResponse,
    ) -> Result<Self::Output, Config::Error>;
}

pub(crate) struct PreparedRead {
    pub(crate) headers: RequestHeaders,
    pub(crate) request: PreparedRequest,
}

pub(crate) struct ReadCall<'call, Config, Operation> {
    pub(crate) config: &'call Config,
    pub(crate) logging: &'call mut dyn AgentLogging,
    pub(crate) litellm_params: &'call Params,
    pub(crate) extra_headers: &'call BTreeMap<String, String>,
    pub(crate) operation: Operation,
}

impl<Config, Operation> ReadCall<'_, Config, Operation>
where
    Config: AgentsApiConfig,
    Operation: ReadOperation<Config>,
{
    pub(crate) fn blocking(
        mut self,
        client: &BlockingClient,
        proxy: Option<&ProxyPolicy>,
    ) -> Result<Operation::Output, AgentRequestError<Config::Error>> {
        let prepared = self.prepared()?;
        let response = send_blocking(client, proxy, &prepared)
            .map_err(|failure| handle_error(self.config, failure))?;
        self.finish(&response)
    }

    pub(crate) fn prepared(&mut self) -> Result<PreparedRead, AgentRequestError<Config::Error>> {
        let base = api_base(self.litellm_params).map_err(AgentRequestError::Parameters)?;
        let headers = self
            .config
            .validate_environment(self.extra_headers, self.litellm_params)
            .map_err(AgentRequestError::Provider)?;
        let request = self
            .operation
            .transform_request(self.config, base, self.litellm_params)
            .map_err(AgentRequestError::Provider)?;
        // LiteLLM logs an empty api key here, since the provider key travels in the headers
        let api_key = SecretString::from("");
        self.logging.pre_call(AgentPreCall {
            input: self.operation.input(),
            api_key: &api_key,
            api_base: &request.url,
            headers: &headers,
            complete_input_dict: None,
        });
        Ok(PreparedRead { headers, request })
    }

    pub(crate) fn finish(
        self,
        response: &UpstreamResponse,
    ) -> Result<Operation::Output, AgentRequestError<Config::Error>> {
        let text = body_text(&response.body);
        self.logging.post_call(AgentPostCall {
            original_response: &text,
            complete_input_dict: None,
        });
        self.operation
            .transform_response(self.config, response)
            .map_err(AgentRequestError::Provider)
    }
}

fn send_blocking(
    client: &BlockingClient,
    proxy: Option<&ProxyPolicy>,
    prepared: &PreparedRead,
) -> Result<UpstreamResponse, RequestFailure> {
    let url = request_url(&prepared.request)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    let sent = client
        .get(url)
        .headers(request_headers(&prepared.headers)?)
        .send()
        .map_err(RequestFailure::Transport)?;
    let status_code = sent.status().as_u16();
    let headers = response_headers(sent.headers());
    let body = read_body(sent, MAX_RESPONSE_BYTES)?;
    Ok(UpstreamResponse {
        status_code,
        headers,
        body,
    })
}
