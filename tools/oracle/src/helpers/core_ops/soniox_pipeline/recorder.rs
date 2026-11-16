use std::collections::VecDeque;
use std::fmt::Display;
use std::future::{Future, ready};
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::api::soniox::prepare::logging::{
    SonioxEvent, SonioxLogger, SonioxPostCall, SonioxPreCall,
};
use betterlite_core::api::soniox::remote::transport::{RemoteTransport, Sleeper};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::api::audio_file_input::{AudioStream, AuthorizedAudioFiles};
use betterlite_openai::openai_base_config::Object;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use super::input::{Answer, Case};
use betterlite_oracle_helper_api::error::HelperError;

pub(super) type Effects = Mutex<Vec<Value>>;
pub(super) type Failure = Box<HelperError>;

struct Scripted {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
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
        })
    }
}

pub(super) struct Recorder<'run> {
    answers: Mutex<VecDeque<Scripted>>,
    names: Vec<String>,
    effects: &'run Effects,
}

impl<'run> Recorder<'run> {
    pub(super) fn new(case: &Case, effects: &'run Effects) -> Result<Self, Failure> {
        let mut scripted = VecDeque::with_capacity(case.answers.len());
        for answer in &case.answers {
            scripted.push_back(Scripted::new(answer)?);
        }
        Ok(Self {
            answers: Mutex::new(scripted),
            names: echoed_names(case),
            effects,
        })
    }

    fn answer(&self) -> Result<UpstreamResponse, TransportError> {
        let pending = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let Some(scripted) = pending else {
            return Err(TransportError::Closed);
        };
        Ok(UpstreamResponse {
            status: scripted.status,
            headers: scripted.headers,
            body: scripted.body,
        })
    }
}

impl RemoteTransport for Recorder<'_> {
    fn call(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        record(&request, &self.names, self.effects);
        ready(self.answer())
    }
}

impl Sleeper for Recorder<'_> {
    fn sleep(&self, _duration: Duration) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

pub(super) struct Logger<'run> {
    pub(super) effects: &'run Effects,
}

impl Logger<'_> {
    fn push(&self, effect: Value) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }
}

impl SonioxLogger for Logger<'_> {
    fn record(&mut self, event: SonioxEvent) -> Result<(), PrePostError> {
        match event {
            SonioxEvent::PreCall(record) => self.pre_call(record),
            SonioxEvent::PostCall(record) => self.post_call(record),
        }
        Ok(())
    }
}

impl Logger<'_> {
    fn pre_call(&self, record: SonioxPreCall) {
        self.push(json!({
            "effect": "pre_call",
            "url": record.url,
            "body": record.body,
        }));
    }

    fn post_call(&self, record: SonioxPostCall) {
        self.push(json!({
            "effect": "post_call",
            "input": record.input,
            "original_response": record.original_response,
            "body": record.body,
        }));
    }
}

pub(super) struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            path.display().to_string(),
        ))
    }
}

pub(super) struct NamedStream {
    name: String,
    content: Vec<u8>,
    read: bool,
}

impl NamedStream {
    pub(super) fn new(filename: &str, content: &str) -> Self {
        Self {
            name: filename.to_owned(),
            content: content.as_bytes().to_vec(),
            read: false,
        }
    }
}

impl AudioStream for NamedStream {
    fn name(&self) -> Option<&str> {
        Some(self.name.as_str())
    }

    fn read(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.read {
            return Ok(Some(Vec::new()));
        }
        self.read = true;
        Ok(Some(self.content.clone()))
    }

    fn tell(&mut self) -> Option<io::Result<u64>> {
        Some(Ok(0))
    }

    fn seek(&mut self, position: u64) -> Option<io::Result<()>> {
        if position == 0 {
            self.read = false;
        }
        Some(Ok(()))
    }
}

fn echoed_names(case: &Case) -> Vec<String> {
    let mut names = vec![String::from("authorization")];
    if let Some(headers) = &case.headers {
        names.extend(headers.keys().map(|name| name.to_ascii_lowercase()));
    }
    names.sort();
    names.dedup();
    names
}

fn record(request: &OutboundRequest, names: &[String], effects: &Effects) {
    let mut view = json!({
        "effect": "request",
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "headers": headers_view(&request.headers, names),
        "timeout_millis": u64::try_from(request.deadline.as_millis()).unwrap_or(u64::MAX),
    });
    let extra = match &request.body {
        RequestBody::Multipart { parts, .. } => Some((
            "files",
            Value::Array(parts.iter().map(part_view).collect::<Vec<Value>>()),
        )),
        RequestBody::Json(bytes) => serde_json::from_slice::<Value>(bytes)
            .ok()
            .map(|body| ("json", body)),
        RequestBody::Empty | RequestBody::Bytes { .. } => None,
    };
    if let (Some((name, value)), Value::Object(fields)) = (extra, &mut view) {
        fields.insert(name.to_owned(), value);
    }
    effects
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(view);
}

fn headers_view(headers: &HeaderMap, names: &[String]) -> Value {
    let mut echoed = Object::new();
    for name in names {
        let value = headers
            .get(name.as_str())
            .and_then(|value| value.to_str().ok())
            .map_or(Value::Null, |text| Value::String(text.to_owned()));
        echoed.insert(name.clone(), value);
    }
    Value::Object(echoed)
}

fn part_view(part: &MultipartPart) -> Value {
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
        class: "SonioxPipelineError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
