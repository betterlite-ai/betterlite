#![cfg(test)]

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_http_fixture::request::Request;
use secrecy::ExposeSecret as _;

use super::logging::{AgentLogging, AgentPostCall, AgentPreCall};
use super::{AgentsApiConfig, CreateAgentRequest, body_text};

pub(crate) use super::requests::{get_request, list_request};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{
    ErrorClasses, Params, PreparedRequest, RequestHeaders, ResponseHeaders, UpstreamResponse,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum FixtureError {
    #[error("refused with status {0}")]
    Refused(u16),
    #[error("the environment was refused")]
    Environment,
}

pub(crate) struct Config {
    calls: Mutex<Vec<String>>,
    pub(crate) headers: RequestHeaders,
    pub(crate) create_url: String,
    pub(crate) create_body: Params,
    pub(crate) list: PreparedRequest,
    pub(crate) item: PreparedRequest,
    pub(crate) refuse_status: Option<u16>,
    pub(crate) refuse_environment: bool,
}

impl Config {
    pub(crate) fn for_address(address: SocketAddr) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            headers: RequestHeaders::new(),
            create_url: format!("http://{address}/v1beta/agents"),
            create_body: Params::new(),
            list: PreparedRequest {
                url: format!("http://{address}/v1beta/agents"),
                params: Params::new(),
            },
            item: PreparedRequest {
                url: format!("http://{address}/v1beta/agents/agent-1"),
                params: Params::new(),
            },
            refuse_status: None,
            refuse_environment: false,
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(_poisoned) => Vec::new(),
        }
    }

    fn record(&self, call: String) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    fn note<T>(&self, call: String, value: T) -> T {
        self.record(call);
        value
    }

    fn answer(&self, response: &UpstreamResponse, label: &str) -> Result<String, FixtureError> {
        if self.refuse_status == Some(response.status_code) {
            return Err(FixtureError::Refused(response.status_code));
        }
        Ok(format!(
            "{label}|{}|{}",
            response.status_code,
            body_text(&response.body)
        ))
    }
}

impl ErrorClasses for Config {
    type Error = FixtureError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> FixtureError {
        self.record(format!("error|{status_code}|{message}|{}", headers.len()));
        FixtureError::Refused(status_code)
    }
}

impl AgentsApiConfig for Config {
    type CreateOutput = String;
    type ListOutput = String;
    type GetOutput = String;

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        litellm_params: &Params,
    ) -> Result<RequestHeaders, FixtureError> {
        self.record(format!(
            "environment|{}|{}",
            headers.len(),
            litellm_params.len()
        ));
        if self.refuse_environment {
            return Err(FixtureError::Environment);
        }
        Ok(self.headers.clone())
    }

    fn get_complete_url(
        &self,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<String, FixtureError> {
        Ok(self.note(format!("create_url|{api_base}"), self.create_url.clone()))
    }

    fn transform_create_request(
        &self,
        name: &str,
        _litellm_params: &Params,
    ) -> Result<Params, FixtureError> {
        Ok(self.note(format!("create_request|{name}"), self.create_body.clone()))
    }

    fn transform_create_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<String, FixtureError> {
        self.answer(response, name)
    }

    fn transform_list_request(
        &self,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(self.note(format!("list_request|{api_base}"), self.list.clone()))
    }

    fn transform_list_response(&self, response: &UpstreamResponse) -> Result<String, FixtureError> {
        self.answer(response, "")
    }

    fn transform_get_request(
        &self,
        name: &str,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(self.note(format!("get_request|{name}|{api_base}"), self.item.clone()))
    }

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<String, FixtureError> {
        self.answer(response, name)
    }
}

#[derive(Default)]
pub(crate) struct Recorder {
    calls: Vec<String>,
}

impl Recorder {
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.clone()
    }
}

impl AgentLogging for Recorder {
    fn pre_call(&mut self, call: AgentPreCall<'_>) {
        let headers = sorted_headers(call.headers);
        let dict = dict_text(call.complete_input_dict);
        self.calls.push(format!(
            "pre|{}|{}|{}|{headers}|{dict}",
            call.input,
            call.api_key.expose_secret(),
            call.api_base
        ));
    }

    fn post_call(&mut self, call: AgentPostCall<'_>) {
        let dict = dict_text(call.complete_input_dict);
        self.calls
            .push(format!("post|{}|{dict}", call.original_response));
    }
}

fn sorted_headers(headers: &RequestHeaders) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}={}", value.expose_secret()))
        .collect::<Vec<String>>()
        .join(";")
}

fn dict_text(dict: Option<&Params>) -> String {
    dict.map_or_else(
        || "-".to_owned(),
        |fields| serde_json::to_string(fields).unwrap_or_default(),
    )
}

pub(crate) fn clients() -> HttpClients {
    HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[]),
    )
}

pub(crate) fn caller_headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn params_at(address: SocketAddr) -> Params {
    base::params(&format!(r#"{{"api_base": "http://{address}"}}"#))
}

pub(crate) fn dead_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

pub(crate) fn create_request<'call>(
    name: &'call str,
    litellm_params: &'call Params,
) -> CreateAgentRequest<'call> {
    CreateAgentRequest {
        name,
        litellm_params,
        extra_headers: caller_headers(&[]),
        extra_body: None,
        timeout: None,
        request_timeout: Duration::from_secs(5),
    }
}

pub(crate) fn sent_body(request: &Request) -> Params {
    serde_json::from_str(&request.body).expect("the request body is JSON")
}
