#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind, Read as _, Result as IoResult, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::interactions::agents_create_read::AgentsApiConfig;
use betterlite_http::interactions::agents_create_read::logging::{
    AgentLogging, AgentPostCall, AgentPreCall,
};
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_http::interactions::base::{
    ErrorClasses, Params, PreparedRequest, RequestHeaders, ResponseHeaders, UpstreamResponse,
};
use betterlite_models::controls::json_value::JsonValue;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

const TIMEOUT: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_millis(10);

// the provider error a fixture config raises; LiteLLM's exception_type reads message and status_code off it
#[derive(Debug, Error)]
#[error("{message}")]
pub(crate) struct ProviderRefusal {
    pub(crate) message: String,
    pub(crate) status: Option<u16>,
}

// a config that puts every litellm param on the wire, so a test reads back exactly what the collection prepared
pub(crate) struct FixtureConfig {
    headers: RequestHeaders,
}

impl FixtureConfig {
    pub(crate) fn gemini_headers() -> Self {
        let mut headers = RequestHeaders::new();
        headers.insert(
            "content-type".to_owned(),
            SecretString::from("application/json"),
        );
        headers.insert("x-goog-api-key".to_owned(), SecretString::from("key-1"));
        Self { headers }
    }
}

impl ErrorClasses for FixtureConfig {
    type Error = ProviderRefusal;

    fn get_error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: &ResponseHeaders,
    ) -> ProviderRefusal {
        ProviderRefusal {
            message: message.to_owned(),
            status: Some(status_code),
        }
    }
}

impl AgentsApiConfig for FixtureConfig {
    type CreateOutput = String;
    type ListOutput = String;
    type GetOutput = String;

    fn validate_environment(
        &self,
        headers: &BTreeMap<String, String>,
        _litellm_params: &Params,
    ) -> Result<RequestHeaders, ProviderRefusal> {
        let mut merged = self.headers.clone();
        for (name, value) in headers {
            merged.insert(name.clone(), SecretString::from(value.clone()));
        }
        Ok(merged)
    }

    fn get_complete_url(
        &self,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<String, ProviderRefusal> {
        Ok(format!("{api_base}/v1beta/agents"))
    }

    fn transform_create_request(
        &self,
        name: &str,
        litellm_params: &Params,
    ) -> Result<Params, ProviderRefusal> {
        let mut body = litellm_params.clone();
        body.insert("name".to_owned(), JsonValue::Text(name.to_owned()));
        Ok(body)
    }

    fn transform_create_response(
        &self,
        response: &UpstreamResponse,
        _name: &str,
    ) -> Result<String, ProviderRefusal> {
        decoded(response)
    }

    fn transform_list_request(
        &self,
        api_base: &str,
        litellm_params: &Params,
    ) -> Result<PreparedRequest, ProviderRefusal> {
        let mut params = Params::new();
        if let Some(size) = litellm_params.get("page_size").and_then(query_text) {
            params.insert("pageSize".to_owned(), JsonValue::Text(size));
        }
        if let Some(token) = litellm_params.get("page_token").and_then(query_text) {
            params.insert("pageToken".to_owned(), JsonValue::Text(token));
        }
        Ok(PreparedRequest {
            url: format!("{api_base}/v1beta/agents"),
            params,
        })
    }

    fn transform_list_response(
        &self,
        response: &UpstreamResponse,
    ) -> Result<String, ProviderRefusal> {
        decoded(response)
    }

    fn transform_get_request(
        &self,
        name: &str,
        api_base: &str,
        _litellm_params: &Params,
    ) -> Result<PreparedRequest, ProviderRefusal> {
        Ok(PreparedRequest {
            url: format!("{api_base}/v1beta/agents/{name}"),
            params: Params::new(),
        })
    }

    fn transform_get_response(
        &self,
        response: &UpstreamResponse,
        _name: &str,
    ) -> Result<String, ProviderRefusal> {
        decoded(response)
    }
}

// Gemini's pagination drops a zero page size and an empty token before the query is built
fn query_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Text(text) if !text.is_empty() => Some(text.clone()),
        JsonValue::Number(number) if number.to_string() != "0" => Some(number.to_string()),
        JsonValue::Text(_)
        | JsonValue::Number(_)
        | JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

// the real Gemini config raises its provider error for a failing status, as LiteLLM's _raise_for_status does
fn decoded(response: &UpstreamResponse) -> Result<String, ProviderRefusal> {
    if !(200..300).contains(&response.status_code) {
        return Err(ProviderRefusal {
            message: str::from_utf8(&response.body)
                .unwrap_or_default()
                .to_owned(),
            status: Some(response.status_code),
        });
    }
    str::from_utf8(&response.body)
        .map(str::to_owned)
        .map_err(|_utf8| ProviderRefusal {
            message: "the provider reply is not UTF-8".to_owned(),
            status: None,
        })
}

#[derive(Default)]
pub(crate) struct Recorder {
    calls: Vec<String>,
}

impl Recorder {
    pub(crate) fn calls(&self) -> &[String] {
        &self.calls
    }
}

impl AgentLogging for Recorder {
    fn pre_call(&mut self, call: AgentPreCall<'_>) {
        let headers = sorted_headers(call.headers);
        let dict = dict_text(call.complete_input_dict);
        self.calls.push(format!(
            "pre|{}|{}|{headers}|{dict}",
            call.input, call.api_base
        ));
    }

    fn post_call(&mut self, call: AgentPostCall<'_>) {
        self.calls.push(format!("post|{}", call.original_response));
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

// a one-request-at-a-time HTTP server the collection's transport can reach over loopback
pub(crate) struct Provider {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Provider {
    pub(crate) fn start(status: u16, body: &str, delay: Option<Duration>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the fixture binds");
        let address = listener
            .local_addr()
            .expect("the fixture address is readable");
        let answer = reply(status, body);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let recorded = Arc::clone(&requests);
        let halted = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .spawn(move || serve(&listener, &answer, delay, &recorded, &halted))
            .expect("the fixture thread starts");
        Self {
            address,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        match self.requests.lock() {
            Ok(requests) => requests.clone(),
            Err(_poisoned) => Vec::new(),
        }
    }

    pub(crate) fn sent(&self) -> String {
        self.requests()
            .into_iter()
            .next()
            .expect("the fixture received a request")
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        drop(self.worker.take());
    }
}

fn serve(
    listener: &TcpListener,
    answer: &[u8],
    delay: Option<Duration>,
    requests: &Mutex<Vec<String>>,
    stop: &AtomicBool,
) {
    listener
        .set_nonblocking(true)
        .expect("the fixture listener is nonblocking");
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _peer)) => {
                stream
                    .set_nonblocking(false)
                    .expect("the fixture stream blocks");
                stream
                    .set_read_timeout(Some(TIMEOUT))
                    .expect("the fixture stream times out");
                let received = read_request(&mut stream).expect("the fixture reads the request");
                if let Ok(mut recorded) = requests.lock() {
                    recorded.push(received);
                }
                if let Some(pause) = delay {
                    thread::park_timeout(pause);
                }
                drop(stream.write_all(answer));
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => thread::park_timeout(RETRY),
            Err(error) => panic!("the fixture listener failed: {error}"),
        }
    }
}

fn reply(status: u16, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} Fixture\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
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
    HttpClients::new(policy(), proxy_policy())
}

fn policy() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-test/0".to_owned(),
    }
}

fn proxy_policy() -> Arc<ProxyPolicy> {
    let environment = BTreeMap::new();
    Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
            .expect("the empty proxy environment resolves"),
    )
}

pub(crate) fn sent_body(sent: &str) -> Params {
    let (_head, body) = sent
        .split_once("\r\n\r\n")
        .expect("the request has a head and a body");
    serde_json::from_str(body).expect("the request body is JSON")
}
