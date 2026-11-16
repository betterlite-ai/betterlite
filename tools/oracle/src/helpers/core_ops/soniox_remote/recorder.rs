use std::collections::VecDeque;
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::api::soniox::remote::transport::{RemoteTransport, Sleeper};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::openai_base_config::{Headers, Object};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use super::input::Answer;
use betterlite_oracle_helper_api::error::HelperError;

pub(super) type Failure = Box<HelperError>;

struct Scripted {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    fail: bool,
}

impl Scripted {
    fn new(answer: &Answer) -> Result<Self, Failure> {
        let mut headers = HeaderMap::new();
        for (name, value) in &answer.headers {
            let header_name = HeaderName::try_from(name.as_str()).map_err(failure)?;
            let header_value = HeaderValue::try_from(value.as_str()).map_err(failure)?;
            headers.insert(header_name, header_value);
        }
        Ok(Self {
            status: answer.status,
            headers,
            body: answer.body.as_bytes().to_vec(),
            fail: answer.fail,
        })
    }
}

pub(super) struct Recorder {
    answers: Mutex<VecDeque<Scripted>>,
    auth_headers: Headers,
    calls: Mutex<Vec<Value>>,
    sleeps: Mutex<Vec<u64>>,
}

impl Recorder {
    pub(super) fn new(answers: &[Answer], auth_headers: &Headers) -> Result<Self, Failure> {
        let mut scripted = VecDeque::with_capacity(answers.len());
        for answer in answers {
            scripted.push_back(Scripted::new(answer)?);
        }
        Ok(Self {
            answers: Mutex::new(scripted),
            auth_headers: auth_headers.clone(),
            calls: Mutex::new(Vec::new()),
            sleeps: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn sleeps(&self) -> Vec<u64> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn reply(&self) -> Result<UpstreamResponse, TransportError> {
        let pending = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(scripted) = pending else {
            return Err(TransportError::Closed);
        };
        if scripted.fail {
            return Err(TransportError::Closed);
        }
        Ok(UpstreamResponse {
            status: scripted.status,
            headers: scripted.headers,
            body: scripted.body,
        })
    }
}

impl RemoteTransport for Recorder {
    fn call(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record(&request, &self.auth_headers));
        ready(self.reply())
    }
}

impl Sleeper for Recorder {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX));
        ready(())
    }
}

fn record(request: &OutboundRequest, auth_headers: &Headers) -> Value {
    let mut recorded = json!({
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "headers": header_echo(&request.headers, auth_headers),
        "timeout_ms": u64::try_from(request.deadline.as_millis()).unwrap_or(u64::MAX),
    });
    if let RequestBody::Multipart { parts, .. } = &request.body {
        let files: Vec<Value> = parts.iter().map(file_part).collect();
        if let Value::Object(fields) = &mut recorded {
            fields.insert("files".to_owned(), Value::Array(files));
        }
    }
    recorded
}

// Every compared header is one of the authenticated names, so a header the source dropped reads as null
fn header_echo(headers: &HeaderMap, expected: &Headers) -> Object {
    let mut echoed = Object::new();
    for name in expected.keys() {
        let lower = name.to_ascii_lowercase();
        let sent = headers
            .get(lower.as_str())
            .and_then(|value| value.to_str().ok())
            .map_or(Value::Null, |text| Value::String(text.to_owned()));
        echoed.insert(lower, sent);
    }
    echoed
}

fn file_part(part: &MultipartPart) -> Value {
    json!({
        "name": part.name,
        "filename": part.file_name,
        "content_type": part.content_type,
        "body": STANDARD.encode(&part.data),
    })
}

pub(super) fn failure<Error>(error: Error) -> Failure
where
    Error: Display,
{
    Box::new(HelperError {
        class: "SonioxRemoteError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
