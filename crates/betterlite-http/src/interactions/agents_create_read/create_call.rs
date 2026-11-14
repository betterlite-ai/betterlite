use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use reqwest::blocking::Client as BlockingClient;
use secrecy::SecretString;
use url::Url;

use super::logging::{AgentLogging, AgentPostCall, AgentPreCall};
use super::{AgentRequestError, AgentsApiConfig, CreateAgentRequest, body_text};
use crate::interactions::base::transport::response::{read_body, response_headers};
use crate::interactions::base::transport::{refuse_unusable_proxy, request_headers};
use crate::interactions::base::{
    MAX_RESPONSE_BYTES, Params, RequestFailure, RequestHeaders, ResponseHeaders, UpstreamResponse,
    api_base, handle_error,
};

pub(crate) struct PreparedCreate {
    pub(crate) headers: RequestHeaders,
    pub(crate) url: String,
    pub(crate) data: Params,
    pub(crate) timeout: Duration,
}

pub(crate) struct CreateCall<'call, Config> {
    pub(crate) config: &'call Config,
    pub(crate) logging: &'call mut dyn AgentLogging,
    pub(crate) request: &'call CreateAgentRequest<'call>,
}

impl<Config> CreateCall<'_, Config>
where
    Config: AgentsApiConfig,
{
    pub(crate) fn blocking(
        mut self,
        client: &BlockingClient,
        proxy: Option<&ProxyPolicy>,
    ) -> Result<Config::CreateOutput, AgentRequestError<Config::Error>> {
        let prepared = self.prepared()?;
        let response = send_blocking(client, proxy, &prepared)
            .map_err(|failure| handle_error(self.config, failure))?;
        self.finish(&prepared, &response)
    }

    pub(crate) fn prepared(&mut self) -> Result<PreparedCreate, AgentRequestError<Config::Error>> {
        let base = api_base(self.request.litellm_params).map_err(AgentRequestError::Parameters)?;
        let headers = self
            .config
            .validate_environment(&self.request.extra_headers, self.request.litellm_params)
            .map_err(AgentRequestError::Provider)?;
        let url = self
            .config
            .get_complete_url(base, self.request.litellm_params)
            .map_err(AgentRequestError::Provider)?;
        let mut data = self
            .config
            .transform_create_request(self.request.name, self.request.litellm_params)
            .map_err(AgentRequestError::Provider)?;
        if let Some(extra) = &self.request.extra_body {
            data.extend(extra.clone());
        }
        // LiteLLM logs an empty api key here, since the provider key travels in the headers
        let api_key = SecretString::from("");
        self.logging.pre_call(AgentPreCall {
            input: self.request.name,
            api_key: &api_key,
            api_base: &url,
            headers: &headers,
            complete_input_dict: Some(&data),
        });
        let timeout = self.request.timeout.unwrap_or(self.request.request_timeout);
        Ok(PreparedCreate {
            headers,
            url,
            data,
            timeout,
        })
    }

    pub(crate) fn finish(
        self,
        prepared: &PreparedCreate,
        response: &UpstreamResponse,
    ) -> Result<Config::CreateOutput, AgentRequestError<Config::Error>> {
        let text = body_text(&response.body);
        self.logging.post_call(AgentPostCall {
            original_response: &text,
            complete_input_dict: Some(&prepared.data),
        });
        self.config
            .transform_create_response(response, self.request.name)
            .map_err(AgentRequestError::Provider)
    }
}

fn send_blocking(
    client: &BlockingClient,
    proxy: Option<&ProxyPolicy>,
    prepared: &PreparedCreate,
) -> Result<UpstreamResponse, RequestFailure> {
    let url = Url::parse(&prepared.url).map_err(RequestFailure::Url)?;
    if let Some(policy) = proxy {
        refuse_unusable_proxy(policy, &url)?;
    }
    let sent = client
        .post(url)
        .headers(request_headers(&prepared.headers)?)
        .json(&prepared.data)
        .timeout(prepared.timeout)
        .send()
        .map_err(RequestFailure::Transport)?;
    let status_code = sent.status().as_u16();
    let headers = response_headers(sent.headers());
    let body = read_body(sent, MAX_RESPONSE_BYTES)?;
    success(status_code, headers, body)
}

// httpx's post raises for a failing status, so _handle_error converts it before any post_call
pub(crate) fn success(
    status_code: u16,
    headers: ResponseHeaders,
    body: Vec<u8>,
) -> Result<UpstreamResponse, RequestFailure> {
    if !(200..300).contains(&status_code) {
        return Err(RequestFailure::Status {
            status_code,
            headers,
            body: body_text(&body),
        });
    }
    Ok(UpstreamResponse {
        status_code,
        headers,
        body,
    })
}
