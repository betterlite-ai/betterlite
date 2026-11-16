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
    ErrorClasses, Logging, Params, RequestHeaders, ResponseHeaders, UpstreamResponse,
};
use crate::interactions::create::{
    CompleteUrlRequest, CreateInteractionConfig, CreateInteractionRequest, TransformRequest,
};
use crate::interactions::get::EnvironmentRequest;
use crate::interactions::streaming::StreamingConfig;
use crate::proxy::StreamResponse;

#[derive(Debug, thiserror::Error)]
pub(super) enum FixtureError {
    #[error("refused with status {0}")]
    Refused(u16),
    #[error("the body is not text")]
    Body,
    #[error("the stream is refused")]
    Stream,
}

#[derive(Default)]
pub(super) struct Fixture {
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) headers: RequestHeaders,
    pub(super) data: Params,
    pub(super) fail_transform_status: Option<u16>,
    pub(super) refuse_stream: bool,
}

impl Fixture {
    // the base a test names one field on, since a struct update over Default::default() is banned
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn record(&self, call: String) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        match self.calls.lock() {
            Ok(calls) => calls.clone(),
            Err(_poisoned) => Vec::new(),
        }
    }

    fn respond(
        &self,
        response: &UpstreamResponse,
        logging: &Logging,
    ) -> Result<String, FixtureError> {
        let input = logging
            .pre_call
            .as_ref()
            .map_or_else(String::new, |call| call.input.clone());
        let request_id = response
            .headers
            .get("x-request-id")
            .map_or("-", String::as_str);
        self.record(format!(
            "response|{}|{input}|{request_id}",
            response.status_code
        ));
        if self.fail_transform_status == Some(response.status_code) {
            return Err(FixtureError::Refused(response.status_code));
        }
        from_utf8(&response.body)
            .map(str::to_owned)
            .map_err(|_error| FixtureError::Body)
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

impl StreamingConfig for Fixture {
    type SyncStream = String;

    type AsyncStream = StreamResponse;

    fn create_sync_stream_iterator(
        &self,
        response: UpstreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<String, FixtureError> {
        let body = from_utf8(&response.body).unwrap_or_default();
        self.record(format!(
            "sync_stream|{}|{}|{body}",
            response.status_code,
            model.unwrap_or("-"),
        ));
        logging
            .model_call_details
            .insert("streamed".to_owned(), JsonValue::Bool(true));
        if self.refuse_stream {
            return Err(FixtureError::Stream);
        }
        from_utf8(&response.body)
            .map(str::to_owned)
            .map_err(|_error| FixtureError::Body)
    }

    fn create_async_stream_iterator(
        &self,
        response: StreamResponse,
        model: Option<&str>,
        logging: &mut Logging,
    ) -> Result<StreamResponse, FixtureError> {
        self.record(format!(
            "async_stream|{}|{}",
            response.response.status().as_u16(),
            model.unwrap_or("-")
        ));
        logging
            .model_call_details
            .insert("streamed".to_owned(), JsonValue::Bool(true));
        if self.refuse_stream {
            return Err(FixtureError::Stream);
        }
        Ok(response)
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
        logging: &mut Logging,
    ) -> Result<String, FixtureError> {
        logging.model_call_details.insert(
            "model".to_owned(),
            JsonValue::Text(model.unwrap_or("-").to_owned()),
        );
        self.respond(response, logging)
    }
}

pub(super) struct Call {
    pub(super) params: Params,
    pub(super) optional_params: Params,
    pub(super) model: Option<String>,
    pub(super) agent: Option<String>,
    pub(super) input: Option<InteractionInput>,
    pub(super) extra_body: Option<Params>,
    pub(super) stream: bool,
    pub(super) provider: ProviderId,
    pub(super) extra_headers: RequestHeaders,
    pub(super) timeout: Option<Duration>,
    pub(super) request_timeout: Duration,
}

impl Call {
    pub(super) fn new(address: SocketAddr) -> Self {
        let mut params = Params::new();
        params.insert(
            "api_base".to_owned(),
            JsonValue::Text(format!("http://{address}")),
        );
        Self {
            params,
            optional_params: Params::new(),
            model: Some("gemini-2.5-flash".to_owned()),
            agent: None,
            input: Some(InteractionInput::Text("hello".to_owned())),
            extra_body: None,
            stream: false,
            provider: ProviderId::Gemini,
            extra_headers: RequestHeaders::new(),
            // the compiler's view of litellm.request_timeout, which defaults to 6000 seconds
            request_timeout: Duration::from_secs(6000),
            timeout: None,
        }
    }

    pub(super) fn request(&self) -> CreateInteractionRequest<'_> {
        CreateInteractionRequest {
            model: self.model.as_deref(),
            agent: self.agent.as_deref(),
            input: self.input.as_ref(),
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

pub(super) fn clients() -> HttpClients {
    HttpClients::new(
        base::policy(Verification::Enabled, None),
        base::proxy_policy(&[]),
    )
}

pub(super) fn dead_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

pub(super) fn caller_headers(entries: &[(&str, &str)]) -> RequestHeaders {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

pub(super) fn api_key_header() -> RequestHeaders {
    caller_headers(&[("x-goog-api-key", "key-1")])
}

pub(super) struct Received {
    pub(super) head: String,
    pub(super) body: String,
}

pub(super) struct Serving {
    pub(super) address: SocketAddr,
    pub(super) request: JoinHandle<Received>,
}

// the shared fixture reads only the head, and these operations have to see the JSON body they posted
pub(super) fn serve_with_body(answer: Vec<u8>, pause: Option<Duration>) -> Serving {
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
    let mut pending = Vec::new();
    while !pending.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        pending.extend_from_slice(&byte);
    }
    let head = String::from_utf8(pending).map_err(IoError::other)?;
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
