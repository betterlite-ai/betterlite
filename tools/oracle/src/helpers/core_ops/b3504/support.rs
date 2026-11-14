use std::collections::{BTreeMap, BTreeSet};
use std::io::{Error as IoError, ErrorKind, Read as _, Result as IoResult, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::agents::collection::{AgentCollection, AgentCollectionFailure};
use betterlite_http::interactions::agents_create_read::logging::{
    AgentLogging, AgentPostCall, AgentPreCall,
};
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use super::gemini::{AgentError, GeminiConfig};
use crate::helpers::HelperError;

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_millis(10);

#[derive(Debug, Deserialize)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

pub(crate) struct Fixture {
    pub(crate) config: GeminiConfig,
    pub(crate) clients: HttpClients,
    pub(crate) logging: LoggingState,
    pub(crate) records: Recorder,
    pub(crate) redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new() -> Result<Self, Box<HelperError>> {
        Ok(Self {
            config: GeminiConfig::new(),
            clients: HttpClients::new(policy(), proxy_policy()?),
            logging: logging_state()?,
            records: Recorder,
            redactor: redactor()?,
        })
    }

    pub(crate) fn collection(&mut self) -> AgentCollection<'_, GeminiConfig> {
        AgentCollection {
            config: &self.config,
            clients: &mut self.clients,
            logging: &mut self.logging,
            records: &mut self.records,
            redactor: &self.redactor,
            request_timeout: Duration::from_secs(600),
        }
    }
}

// the pipeline requires a logger, and the compared answer holds the wire bytes instead of its callbacks
pub(crate) struct Recorder;

impl AgentLogging for Recorder {
    fn pre_call(&mut self, _call: AgentPreCall<'_>) {}

    fn post_call(&mut self, _call: AgentPostCall<'_>) {}
}

pub(crate) fn logging_state() -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "fixture".to_owned(),
            call_type: "fixture".to_owned(),
            start_time: moment()?,
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &redactor()?,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .map_err(helper_failure)
}

fn moment() -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").map_err(helper_failure)
}

pub(crate) fn redactor() -> Result<Redactor, Box<HelperError>> {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(helper_failure)
}

fn policy() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: "betterlite-oracle/0".to_owned(),
    }
}

fn proxy_policy() -> Result<Arc<ProxyPolicy>, Box<HelperError>> {
    let environment = BTreeMap::new();
    ProxyPolicy::resolve(&ProxyTrust::default(), &environment)
        .map(Arc::new)
        .map_err(helper_failure)
}

pub(crate) fn helper_failure<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: "AgentCollectionError".to_owned(),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

// the class carries the status LiteLLM's raised exception reports, and the compared type, param and code stay null
pub(crate) fn raised(error: &AgentCollectionFailure<AgentError>) -> Box<HelperError> {
    Box::new(HelperError {
        class: "AgentCollectionError".to_owned(),
        status: Some(error.class.metadata().status_code),
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message: error.to_string(),
    })
}

pub(crate) fn answer_value(
    server: &Server,
    extra: &BTreeSet<String>,
    result: Value,
    logging: &LoggingState,
) -> Value {
    json!({
        "requests": server.records(extra),
        "result": result,
        "model": logging.model.clone(),
    })
}

// a one-request-at-a-time HTTP server both helpers answer from, so the wire bytes are the compared part
pub(crate) struct Server {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub(crate) fn start(reply: &Reply) -> Result<Self, Box<HelperError>> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(helper_failure)?;
        let address = listener.local_addr().map_err(helper_failure)?;
        let answer = response(reply.status, &reply.body);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let recorded = Arc::clone(&requests);
        let halted = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .spawn(move || serve(&listener, &answer, &recorded, &halted))
            .map_err(helper_failure)?;
        Ok(Self {
            address,
            requests,
            stop,
            worker: Some(worker),
        })
    }

    pub(crate) fn base(&self) -> String {
        format!("http://{}", self.address)
    }

    pub(crate) fn records(&self, extra: &BTreeSet<String>) -> Vec<Value> {
        let received = self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        received
            .iter()
            .map(|sent| request_record(sent, extra))
            .collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        drop(self.worker.take());
    }
}

fn serve(listener: &TcpListener, answer: &[u8], requests: &Mutex<Vec<String>>, stop: &AtomicBool) {
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _peer)) => answer_once(stream, answer, requests),
            Err(error) if error.kind() == ErrorKind::WouldBlock => thread::park_timeout(RETRY),
            Err(_other) => return,
        }
    }
}

fn answer_once(mut stream: TcpStream, answer: &[u8], requests: &Mutex<Vec<String>>) {
    let _blocking = stream.set_nonblocking(false);
    let _deadline = stream.set_read_timeout(Some(READ_TIMEOUT));
    if let Ok(received) = read_request(&mut stream) {
        requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(received);
    }
    drop(stream.write_all(answer));
}

fn response(status: u16, body: &Value) -> Vec<u8> {
    let text = body.to_string();
    format!(
        "HTTP/1.1 {status} Fixture\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
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
        received.extend_from_slice(chunk.get(..read).unwrap_or_default());
        let Some(head_end) = received.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let declared = content_length(received.get(..head_end).unwrap_or_default());
        let body_start = head_end.checked_add(4).unwrap_or(head_end);
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

fn request_record(sent: &str, extra: &BTreeSet<String>) -> Value {
    let (head, body) = sent.split_once("\r\n\r\n").unwrap_or((sent, ""));
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
        .filter(|(name, _value)| kept(name, extra))
        .collect();
    let parsed = serde_json::from_str::<Value>(body).unwrap_or(Value::Null);
    let body = if body.is_empty() { Value::Null } else { parsed };
    json!({
        "method": method,
        "path": path,
        "query": query_map(query),
        "headers": headers,
        "body": body,
    })
}

fn kept(name: &str, extra: &BTreeSet<String>) -> bool {
    name == "content-type" || name == "x-goog-api-key" || extra.contains(name)
}

fn query_map(query: &str) -> BTreeMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}
