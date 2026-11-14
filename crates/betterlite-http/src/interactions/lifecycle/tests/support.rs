use std::io::{Error as IoError, Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::str::{Utf8Error, from_utf8};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use reqwest::blocking::Client as BlockingClient;
use secrecy::SecretString;

use crate::interactions::base::clients::HttpClients;
use crate::interactions::base::tests::support as base;
use crate::interactions::base::{
    ErrorClasses, Logging, Params, PreparedRequest, RequestHeaders, ResponseHeaders,
    UpstreamResponse,
};
use crate::interactions::get::{EnvironmentRequest, InteractionIdRequest};
use crate::interactions::lifecycle::{
    InteractionLifecycleConfig, InteractionsHttpHandler, LifecycleRequest,
};

#[derive(Debug, thiserror::Error)]
pub(super) enum FixtureError {
    #[error("refused with status {0}")]
    Refused(u16),
    #[error("the body is not text")]
    Body(#[source] Utf8Error),
}

#[derive(Default)]
pub(super) struct Fixture {
    pub(super) calls: Mutex<Vec<String>>,
    pub(super) headers: RequestHeaders,
    pub(super) delete_data: Params,
    pub(super) cancel_data: Params,
    pub(super) fail_transform_status: Option<u16>,
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

    fn record_request(&self, tag: &str, request: &InteractionIdRequest<'_>) {
        self.record(format!(
            "{tag}_request|{}|{}|{}",
            request.interaction_id,
            request.headers.len(),
            request.litellm_params.len()
        ));
    }

    fn prepared(
        request: &InteractionIdRequest<'_>,
        params: &Params,
        suffix: &str,
    ) -> PreparedRequest {
        PreparedRequest {
            url: format!(
                "{}/v1beta/interactions/{}{suffix}",
                request.api_base, request.interaction_id
            ),
            params: params.clone(),
        }
    }

    fn respond(
        &self,
        tag: &str,
        response: &UpstreamResponse,
        logging: &Logging,
        interaction_id: Option<&str>,
    ) -> Result<String, FixtureError> {
        let input = logging
            .pre_call
            .as_ref()
            .map_or_else(String::new, |call| call.input.clone());
        let request_id = response
            .headers
            .get("x-request-id")
            .map_or("-", String::as_str);
        let named = interaction_id.map_or_else(String::new, |id| format!("|{id}"));
        self.record(format!(
            "{tag}_response|{}|{input}{named}|{request_id}",
            response.status_code
        ));
        if self.fail_transform_status == Some(response.status_code) {
            return Err(FixtureError::Refused(response.status_code));
        }
        from_utf8(&response.body)
            .map(str::to_owned)
            .map_err(FixtureError::Body)
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

impl InteractionLifecycleConfig for Fixture {
    type DeleteOutput = String;

    type CancelOutput = String;

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

    fn transform_delete_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        self.record_request("delete", request);
        Ok(Self::prepared(request, &self.delete_data, ""))
    }

    fn transform_delete_response(
        &self,
        response: &UpstreamResponse,
        logging: &mut Logging,
        interaction_id: &str,
    ) -> Result<String, FixtureError> {
        self.respond("delete", response, logging, Some(interaction_id))
    }

    fn transform_cancel_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        self.record_request("cancel", request);
        Ok(Self::prepared(request, &self.cancel_data, ":cancel"))
    }

    fn transform_cancel_response(
        &self,
        response: &UpstreamResponse,
        logging: &mut Logging,
    ) -> Result<String, FixtureError> {
        self.respond("cancel", response, logging, None)
    }
}

pub(super) struct Call {
    pub(super) params: Params,
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
            provider: ProviderId::Gemini,
            extra_headers: RequestHeaders::new(),
            timeout: None,
            request_timeout: Duration::from_secs(600),
        }
    }

    pub(super) fn request(&self) -> LifecycleRequest<'_> {
        LifecycleRequest {
            interaction_id: "int_1",
            litellm_params: &self.params,
            provider: self.provider,
            extra_headers: self.extra_headers.clone(),
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

pub(super) fn build_handler<ClientType>(
    fixture: &Fixture,
    supplied: Option<ClientType>,
) -> InteractionsHttpHandler<'_, Fixture, ClientType> {
    InteractionsHttpHandler {
        config: fixture,
        clients: clients(),
        logging: Logging::default(),
        supplied,
    }
}

pub(super) fn sync_handler(
    fixture: &Fixture,
) -> InteractionsHttpHandler<'_, Fixture, BlockingClient> {
    build_handler(fixture, None)
}

pub(super) fn async_handler(
    fixture: &Fixture,
) -> InteractionsHttpHandler<'_, Fixture, reqwest::Client> {
    build_handler(fixture, None)
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

// the shared fixture reads only the head, and these operations have to see whether a request carried a body
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
