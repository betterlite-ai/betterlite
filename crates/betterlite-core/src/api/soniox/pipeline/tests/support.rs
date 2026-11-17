use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, pending, ready};
use std::io::{self, Read};
use std::path::Path;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::api::audio_file_input::{AudioFile, AudioStream, AuthorizedAudioFiles};
use betterlite_openai::openai_base_config::{Headers, Object};
use http::Method;
use http::header::HeaderMap;
use serde_json::{from_slice, to_value};

use crate::api::api_boundaries::ApiLimits;
use crate::api::soniox::pipeline::PipelineCall;
use crate::api::soniox::pipeline::SonioxPipeline;
use crate::api::soniox::pipeline::job::PipelineTarget;
use crate::api::soniox::prepare::logging::{
    SonioxEvent, SonioxLogger, SonioxPostCall, SonioxPreCall,
};
use crate::api::soniox::records::response::SonioxTranscriptionResponse;
use crate::api::soniox::remote::transport::{RemoteTransport, Sleeper};

pub(super) const API_BASE: &str = "https://api.soniox.test";
pub(super) const BOUNDARY: &str = "boundary-7";
pub(super) const TIMEOUT: Duration = Duration::from_millis(2500);
pub(super) const KEY: &str = "sk-local";
pub(super) const MODEL: &str = "soniox/stt-async-v4";
pub(super) const AUDIO_NAME: &str = "audio.mp3";
pub(super) const AUDIO_CONTENT: &[u8] = b"RIFF-data";
pub(super) const UPLOAD_URL: &str = "https://api.soniox.test/v1/files";
pub(super) const TRANSCRIPTIONS_URL: &str = "https://api.soniox.test/v1/transcriptions";

pub(super) static EMPTY_ENVIRONMENT: BTreeMap<String, String> = BTreeMap::new();
static NO_FILES: NoFiles = NoFiles;

pub(super) enum Reply {
    Answer { status: u16, body: String },
    Fail(TransportError),
    Hang,
}

type BoxFuture = Pin<Box<dyn Future<Output = Result<UpstreamResponse, TransportError>> + Send>>;

#[derive(Clone)]
pub(super) enum RecordedBody {
    Empty,
    Json(JsonValue),
    Multipart { parts: Vec<MultipartPart> },
}

#[derive(Clone)]
pub(super) struct Call {
    pub(super) method: Method,
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: RecordedBody,
    pub(super) deadline: Duration,
}

#[derive(Clone)]
pub(super) enum LogRecord {
    PreCall {
        url: String,
        body: JsonValue,
    },
    PostCall {
        input: JsonValue,
        original: JsonValue,
    },
}

#[derive(Default)]
pub(super) struct Effects {
    kinds: Mutex<Vec<&'static str>>,
}

impl Effects {
    pub(super) fn kinds(&self) -> Vec<&'static str> {
        self.kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
pub(super) struct LogBook {
    records: Mutex<Vec<LogRecord>>,
}

impl LogBook {
    pub(super) fn records(&self) -> Vec<LogRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(super) struct Recorder<'log> {
    answers: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<Call>>,
    sleeps: Mutex<Vec<Duration>>,
    effects: &'log Effects,
}

impl<'log> Recorder<'log> {
    pub(super) fn new(answers: Vec<Reply>, effects: &'log Effects) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from(answers)),
            calls: Mutex::new(Vec::new()),
            sleeps: Mutex::new(Vec::new()),
            effects,
        }
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn sleeps(&self) -> Vec<Duration> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RemoteTransport for Recorder<'_> {
    fn call(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.effects
            .kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("request");
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record(request));
        let next_answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let answer: BoxFuture = match next_answer {
            Some(Reply::Answer { status, body }) => {
                let response = UpstreamResponse {
                    status,
                    headers: HeaderMap::new(),
                    body: body.into_bytes(),
                };
                Box::pin(ready(Ok(response)))
            }
            Some(Reply::Fail(error)) => Box::pin(ready(Err(error))),
            Some(Reply::Hang) => Box::pin(pending::<Result<UpstreamResponse, TransportError>>()),
            None => Box::pin(ready(Err(TransportError::Closed))),
        };
        answer
    }
}

impl Sleeper for Recorder<'_> {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.effects
            .kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push("sleep");
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
        ready(())
    }
}

pub(super) struct Logger<'log> {
    effects: &'log Effects,
    book: &'log LogBook,
}

impl<'log> Logger<'log> {
    pub(super) fn new(effects: &'log Effects, book: &'log LogBook) -> Self {
        Self { effects, book }
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
    fn push(&self, kind: &'static str, record: LogRecord) {
        self.effects
            .kinds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(kind);
        self.book
            .records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }

    fn pre_call(&self, record: SonioxPreCall) {
        let body = to_value(record.body).expect("the pre-call body is JSON");
        self.push(
            "pre_call",
            LogRecord::PreCall {
                url: record.url,
                body,
            },
        );
    }

    fn post_call(&self, record: SonioxPostCall) {
        let input = to_value(record.input).expect("the post-call input is JSON");
        let original = to_value(record.original_response).expect("the post-call original is JSON");
        self.push("post_call", LogRecord::PostCall { input, original });
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
    pub(super) fn new(name: &str, content: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            content: content.to_vec(),
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

pub(super) fn headers() -> Headers {
    let mut headers = Headers::new();
    headers.insert("Authorization".to_owned(), format!("Bearer {KEY}"));
    headers.insert("x-client".to_owned(), "suite".to_owned());
    headers
}

pub(super) fn params(entries: &[(&str, JsonValue)]) -> Object {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(super) fn call(params: &Object, atranscription: bool) -> PipelineCall<'_> {
    PipelineCall {
        model: MODEL,
        optional_params: params,
        headers: headers(),
        api_key: Some(KEY),
        api_base: Some(API_BASE),
        environment: &EMPTY_ENVIRONMENT,
        timeout: TIMEOUT,
        atranscription,
    }
}

pub(super) fn pipeline_with<'deps, P, S>(
    provider: &'deps P,
    sleeper: &'deps S,
) -> SonioxPipeline<'deps, P, S> {
    let files: &'deps (dyn AuthorizedAudioFiles + Sync) = &NO_FILES;
    SonioxPipeline {
        provider,
        sleeper,
        audio_files: files,
        limits: ApiLimits::default(),
        boundaries: || BOUNDARY.to_owned(),
    }
}

pub(super) fn target<'call, 'file>(
    audio_file: Option<&'call mut AudioFile<'file>>,
    response: &'call mut SonioxTranscriptionResponse,
    logger: &'call mut (dyn SonioxLogger + Send),
) -> PipelineTarget<'call, 'file> {
    PipelineTarget {
        audio_file,
        response,
        logger,
    }
}

pub(super) fn response() -> SonioxTranscriptionResponse {
    SonioxTranscriptionResponse {
        text: Some(String::new()),
        usage: None,
        extra: Object::new(),
        hidden_params: Object::new(),
    }
}

pub(super) fn answer(status: u16, body: &str) -> Reply {
    Reply::Answer {
        status,
        body: body.to_owned(),
    }
}

pub(super) fn sent<'call>(call: &'call Call, name: &str) -> Option<&'call str> {
    call.headers.get(name)?.to_str().ok()
}

pub(super) fn json_body(body: &RecordedBody) -> &JsonValue {
    match body {
        RecordedBody::Json(value) => value,
        RecordedBody::Empty | RecordedBody::Multipart { .. } => {
            panic!("the pipeline call sends a JSON body")
        }
    }
}

pub(super) fn multipart_parts(body: &RecordedBody) -> &[MultipartPart] {
    match body {
        RecordedBody::Multipart { parts } => parts,
        RecordedBody::Empty | RecordedBody::Json(_) => {
            panic!("the pipeline call sends a multipart body")
        }
    }
}

fn record(request: OutboundRequest) -> Call {
    let body = match request.body {
        RequestBody::Empty => RecordedBody::Empty,
        RequestBody::Json(bytes) => {
            RecordedBody::Json(from_slice(&bytes).expect("the create body is JSON"))
        }
        RequestBody::Multipart { parts, .. } => RecordedBody::Multipart { parts },
        RequestBody::Bytes { .. } => panic!("the pipeline sends no raw bytes body"),
    };
    Call {
        method: request.method,
        url: request.url.to_string(),
        headers: request.headers,
        body,
        deadline: request.deadline,
    }
}
