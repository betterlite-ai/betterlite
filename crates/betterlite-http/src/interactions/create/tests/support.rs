use std::io::{Error as IoError, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str::from_utf8;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_interactions::types::interaction::InteractionInput;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use secrecy::SecretString;

use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{
    ClientError, ErrorClasses, Logging, Params, RequestFailure, RequestHeaders, ResponseHeaders,
    UpstreamResponse,
};
use crate::interactions::create::asynchronous::async_create_interaction;
use crate::interactions::create::synchronous::create_interaction;
use crate::interactions::create::{
    CompleteUrlRequest, CreateInteractionConfig, CreateInteractionError, CreateInteractionRequest,
    CreateOutcome, TransformRequest,
};
use crate::interactions::get::EnvironmentRequest;
use crate::interactions::streaming::SyncStreamingConfig;
use crate::proxy::StreamResponse;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FixtureError {
    #[error("refused with status {0}")]
    Refused(u16),
    #[error("the body is not text")]
    Body,
    #[error("the stream is refused")]
    Stream,
}

pub(crate) type Failure = CreateInteractionError<FixtureError>;

pub(crate) type SyncResult = Result<CreateOutcome<String, String>, Failure>;

pub(crate) type AsyncResult = Result<CreateOutcome<String, StreamResponse>, Failure>;

pub(crate) fn provider_error<Output>(result: Result<Output, Failure>) -> FixtureError {
    match result.unwrap_err() {
        Failure::Provider(error) => error,
        other => panic!("expected a provider error, got {other:?}"),
    }
}

pub(crate) fn transport_error<Output>(result: Result<Output, Failure>) -> RequestFailure {
    match result.unwrap_err() {
        Failure::Transport(failure) => failure,
        other => panic!("expected a transport error, got {other:?}"),
    }
}

pub(crate) fn client_error<Output>(result: Result<Output, Failure>) -> ClientError {
    match result.unwrap_err() {
        Failure::Client(error) => error,
        other => panic!("expected a client error, got {other:?}"),
    }
}

#[derive(Default)]
pub(crate) struct Fixture {
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) headers: RequestHeaders,
    pub(crate) data: Params,
    pub(crate) fail_transform_status: Option<u16>,
    pub(crate) refuse_stream: bool,
}

impl Fixture {
    // the base a test names one field on, since a struct update over Default::default() is banned
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn record(&self, call: String) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .map_or_else(|_poisoned| Vec::new(), |calls| calls.clone())
    }

    fn respond(&self, response: &UpstreamResponse) -> Result<String, FixtureError> {
        if self.fail_transform_status == Some(response.status_code) {
            return Err(FixtureError::Refused(response.status_code));
        }
        from_utf8(&response.body)
            .map(str::to_owned)
            .map_err(|_error| FixtureError::Body)
    }

    pub(crate) fn note_stream(
        &self,
        kind: &str,
        status_code: u16,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<(), FixtureError> {
        self.record(format!("{kind}|{status_code}|{}", model.unwrap_or("-")));
        logging
            .model_call_details
            .insert("streamed".to_owned(), JsonValue::Bool(true));
        if self.refuse_stream {
            return Err(FixtureError::Stream);
        }
        Ok(())
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

impl SyncStreamingConfig for Fixture {
    type SyncStream = String;

    fn create_sync_stream_iterator(
        &self,
        response: UpstreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<String, FixtureError> {
        let body = from_utf8(&response.body).map_err(|_error| FixtureError::Body)?;
        self.note_stream("sync_stream", response.status_code, model, logging)?;
        Ok(body.to_owned())
    }
}

impl CreateInteractionConfig for Fixture {
    type Output = String;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, FixtureError> {
        self.record(format!(
            "environment|{}|{}|{}",
            request.model,
            request.headers.len(),
            request.litellm_params.map_or(0, Params::len)
        ));
        Ok(self.headers.clone())
    }

    fn get_complete_url(&self, request: &CompleteUrlRequest<'_>) -> Result<String, FixtureError> {
        self.record(format!(
            "url|{}|{}|{}|{}|{}",
            request.api_base,
            request.model.unwrap_or("-"),
            request.agent.unwrap_or("-"),
            request.litellm_params.len(),
            request.stream
        ));
        Ok(format!("{}/v1beta/interactions", request.api_base))
    }

    fn transform_request(&self, request: &TransformRequest<'_>) -> Result<Params, FixtureError> {
        self.record(format!(
            "create|{}|{}|{}|{}|{}|{}",
            request.model.unwrap_or("-"),
            request.agent.unwrap_or("-"),
            request.input.is_some(),
            request.optional_params.len(),
            request.litellm_params.len(),
            request.headers.len()
        ));
        Ok(self.data.clone())
    }

    fn transform_response(
        &self,
        model: Option<&str>,
        response: &UpstreamResponse,
        _logging: &mut Logging,
    ) -> Result<String, FixtureError> {
        self.record(format!("response|{}", response.status_code));
        self.record(format!("model|{}", model.unwrap_or("-")));
        self.respond(response)
    }
}

pub(crate) struct Call {
    pub(crate) params: Params,
    pub(crate) optional_params: Params,
    pub(crate) model: Option<String>,
    pub(crate) agent: Option<String>,
    pub(crate) input: Option<InteractionInput>,
    pub(crate) extra_body: Option<Params>,
    pub(crate) stream: bool,
    pub(crate) provider: ProviderId,
    pub(crate) extra_headers: RequestHeaders,
    pub(crate) timeout: Option<Duration>,
    pub(crate) request_timeout: Duration,
}

impl Call {
    pub(crate) fn new(address: SocketAddr) -> Self {
        let api_base = JsonValue::Text(format!("http://{address}"));
        let params = Params::from([("api_base".to_owned(), api_base)]);
        let model = Some("gemini-2.5-flash".to_owned());
        let input = Some(InteractionInput::Text("hello".to_owned()));
        Self {
            params,
            optional_params: Params::new(),
            model,
            agent: None,
            input,
            extra_body: None,
            stream: false,
            provider: ProviderId::Gemini,
            extra_headers: RequestHeaders::new(),
            // the compiler's view of litellm.request_timeout, which defaults to 6000 seconds
            request_timeout: Duration::from_secs(6000),
            timeout: None,
        }
    }

    pub(crate) fn request(&self) -> CreateInteractionRequest<'_> {
        let model = self.model.as_deref();
        let agent = self.agent.as_deref();
        let input = self.input.as_ref();
        CreateInteractionRequest {
            model,
            agent,
            input,
            optional_params: &self.optional_params,
            litellm_params: &self.params,
            provider: self.provider,
            stream: self.stream,
            extra_headers: self.extra_headers.clone(),
            extra_body: self.extra_body.clone(),
            timeout: self.timeout,
            request_timeout: self.request_timeout,
        }
    }
}

pub(crate) fn run(fixture: &Fixture, call: &Call) -> (SyncResult, Logging) {
    let mut clients = clients();
    let mut logging = Logging::default();
    let outcome = create_interaction(fixture, &mut clients, &mut logging, None, &call.request());
    (outcome, logging)
}

pub(crate) async fn run_async(fixture: &Fixture, call: &Call) -> (AsyncResult, Logging) {
    let mut clients = clients();
    let mut logging = Logging::default();
    let outcome =
        async_create_interaction(fixture, &mut clients, &mut logging, None, &call.request()).await;
    (outcome, logging)
}

pub(crate) fn clients() -> HttpClients {
    HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[]),
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

pub(crate) fn api_key_header() -> RequestHeaders {
    caller_headers(&[("x-goog-api-key", "key-1")])
}

pub(crate) fn posted(body: &str) -> JsonValue {
    serde_json::from_str(body).unwrap()
}

pub(crate) struct Received {
    pub(crate) head: String,
    pub(crate) body: String,
}

pub(crate) struct Serving {
    pub(crate) address: SocketAddr,
    pub(crate) request: JoinHandle<Received>,
}

// the shared fixture reads only the head, and these operations have to see the JSON body they posted
pub(crate) fn serve_with_body(answer: Vec<u8>, pause: Option<Duration>) -> Serving {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request = thread::Builder::new()
        .spawn(move || {
            let (mut stream, _peer) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
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

fn read_request(stream: &mut TcpStream) -> Result<Received, IoError> {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).map_err(IoError::other)?;
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(Received {
        head,
        body: String::from_utf8(body).map_err(IoError::other)?,
    })
}
