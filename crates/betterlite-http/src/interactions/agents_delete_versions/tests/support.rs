use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::env_integer::timeout::RequestTimeout;
use betterlite_config::tls::Verification;
use betterlite_models::controls::json_value::JsonValue;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;

use super::{AgentRequest, AgentsConfig, EnvironmentRequest, NamedAgentRequest};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as fixture;
use crate::interactions::base::{
    ErrorClasses, Params, PreparedRequest, RequestHeaders, ResponseHeaders, UpstreamResponse,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum FixtureError {
    #[error("refused with status {0}")]
    Refused(u16),
    #[error("the {0} are not usable")]
    Field(&'static str),
}

#[derive(Default)]
pub(crate) struct Fixture {
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) headers: RequestHeaders,
    pub(crate) query: Params,
    pub(crate) fail_environment: bool,
}

impl Fixture {
    fn record(&self, call: String) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(_poisoned) => Vec::new(),
        }
    }
}

impl ErrorClasses for Fixture {
    type Error = FixtureError;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> FixtureError {
        let retry_after = headers.get("retry-after").map_or("-", String::as_str);
        self.record(format!("error|{status_code}|{message}|{retry_after}"));
        FixtureError::Refused(status_code)
    }
}

impl AgentsConfig for Fixture {
    type DeleteOutput = String;
    type VersionsOutput = String;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, FixtureError> {
        self.record(format!(
            "environment|{}|{}",
            request.headers.len(),
            request.litellm_params.len()
        ));
        if self.fail_environment {
            return Err(FixtureError::Field("headers"));
        }
        Ok(self.headers.clone())
    }

    fn transform_delete_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<String, FixtureError> {
        self.record(format!(
            "delete_request|{}|{}|{}",
            request.name,
            request.api_base,
            request.litellm_params.len()
        ));
        Ok(format!(
            "{}/v1beta/agents/{}",
            request.api_base, request.name
        ))
    }

    fn transform_delete_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<String, FixtureError> {
        self.record(format!("delete_response|{name}|{}", response.status_code));
        let size = response.body.len();
        Ok(format!("delete|{name}|{}|{size}", response.status_code))
    }

    fn transform_versions_request(
        &self,
        request: &NamedAgentRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        self.record(format!(
            "versions_request|{}|{}|{}",
            request.name,
            request.api_base,
            request.litellm_params.len()
        ));
        Ok(PreparedRequest {
            url: format!(
                "{}/v1beta/agents/{}/versions",
                request.api_base, request.name
            ),
            params: self.query.clone(),
        })
    }

    fn transform_versions_response(
        &self,
        response: &UpstreamResponse,
        name: &str,
    ) -> Result<String, FixtureError> {
        self.record(format!("versions_response|{name}|{}", response.status_code));
        let size = response.body.len();
        Ok(format!("versions|{name}|{}|{size}", response.status_code))
    }
}

pub(crate) struct Call {
    pub(crate) name: String,
    pub(crate) params: Params,
    pub(crate) extra_headers: RequestHeaders,
    pub(crate) timeout: Option<Duration>,
    pub(crate) request_timeout: RequestTimeout,
}

impl Call {
    pub(crate) fn new(address: SocketAddr) -> Self {
        let mut params = Params::new();
        params.insert(
            "api_base".to_owned(),
            JsonValue::Text(format!("http://{address}")),
        );
        Self {
            name: "agent-1".to_owned(),
            params,
            extra_headers: RequestHeaders::new(),
            timeout: None,
            request_timeout: resolved_timeout(&[]),
        }
    }

    pub(crate) fn agent_request(&self) -> AgentRequest<'_> {
        AgentRequest {
            name: &self.name,
            litellm_params: &self.params,
            extra_headers: self.extra_headers.clone(),
            timeout: self.timeout,
            request_timeout: self.request_timeout,
        }
    }
}

pub(crate) const fn failing_environment() -> Fixture {
    Fixture {
        calls: Mutex::new(Vec::new()),
        headers: RequestHeaders::new(),
        query: Params::new(),
        fail_environment: true,
    }
}

pub(crate) fn resolved_timeout(variables: &[(&str, &str)]) -> RequestTimeout {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RequestTimeout::resolve(&environment).unwrap()
}

pub(crate) fn clients() -> HttpClients {
    HttpClients::new(
        fixture::policy(Verification::Enabled, None),
        fixture::proxy_policy(&[]),
    )
}

pub(crate) fn dead_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

pub(crate) fn caller_headers(entries: &[(&str, &str)]) -> RequestHeaders {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

pub(crate) fn reply_with_bytes(status: &str, body: &[u8]) -> Vec<u8> {
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let mut answer = head.into_bytes();
    answer.extend_from_slice(body);
    answer
}

pub(crate) fn exposed(headers: &RequestHeaders, name: &str) -> String {
    headers
        .get(name)
        .map(|value| value.expose_secret().to_owned())
        .unwrap_or_default()
}
