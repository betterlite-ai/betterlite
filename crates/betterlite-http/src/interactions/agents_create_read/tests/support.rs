#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind, Read as _, Result as IoResult, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use betterlite_config::tls::Verification;
use secrecy::ExposeSecret as _;

use super::logging::{AgentLogging, AgentPostCall, AgentPreCall};
use super::{AgentsApiConfig, CreateAgentRequest, body_text};

pub(crate) use super::requests::{get_request, list_request};
use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{
    ErrorClasses, Params, PreparedRequest, RequestHeaders, ResponseHeaders, UpstreamResponse,
};

const TIMEOUT: Duration = Duration::from_secs(5);
const ATTEMPTS: u32 = 500;
const RETRY: Duration = Duration::from_millis(10);

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

pub(crate) struct Serving {
    pub(crate) address: SocketAddr,
    pub(crate) request: thread::JoinHandle<String>,
}

pub(crate) fn serve_post(answer: Vec<u8>, pause: Option<Duration>) -> Serving {
    let listener = base::listener();
    let address = listener.local_addr().unwrap();
    let request = thread::Builder::new()
        .spawn(move || {
            let mut stream = connected(&listener);
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(TIMEOUT)).unwrap();
            let received = read_request(&mut stream).unwrap();
            if let Some(delay) = pause {
                thread::park_timeout(delay);
            }
            drop(stream.write_all(&answer));
            received
        })
        .unwrap();
    Serving { address, request }
}

fn connected(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    (0..ATTEMPTS)
        .find_map(|_| match listener.accept() {
            Ok((stream, _peer)) => Some(stream),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::park_timeout(RETRY);
                None
            }
            Err(error) => panic!("the fixture listener failed: {error}"),
        })
        .expect("the client never connected to the fixture")
}

fn read_request(stream: &mut TcpStream) -> IoResult<String> {
    let mut received = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(IoError::from(ErrorKind::UnexpectedEof));
        }
        received.extend_from_slice(&chunk[..read]);
        let Some(head_end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let declared = content_length(received.get(..head_end).unwrap_or_default());
        let body_start = head_end
            .checked_add(4)
            .expect("the request head fits in memory");
        if received.len().saturating_sub(body_start) >= declared {
            return String::from_utf8(received).map_err(IoError::other);
        }
    }
}

fn content_length(head: &[u8]) -> usize {
    let Ok(text) = str::from_utf8(head) else {
        return 0;
    };
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| {
            if name.eq_ignore_ascii_case("content-length") {
                return value.trim().parse().ok();
            }
            None
        })
        .unwrap_or(0)
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

pub(crate) fn sent_body(sent: &str) -> Params {
    let (_head, body) = sent
        .split_once("\r\n\r\n")
        .expect("the request has a head and a body");
    serde_json::from_str(body).expect("the request body is JSON")
}
