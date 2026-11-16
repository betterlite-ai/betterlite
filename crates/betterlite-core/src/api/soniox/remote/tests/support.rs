use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::{Headers, Object};
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use rust_decimal::Decimal;

use crate::api::api_boundaries::ApiLimits;
use crate::api::soniox::prepare::Prepared;
use crate::api::soniox::prepare::options::HandlerOptions;
use crate::api::soniox::remote::SonioxRemote;
use crate::api::soniox::remote::transport::{RemoteTransport, Sleeper};

pub const API_BASE: &str = "https://api.soniox.test";
pub const BOUNDARY: &str = "boundary-7";
pub const TIMEOUT: Duration = Duration::from_millis(2500);

pub enum Outcome {
    Reply {
        status: u16,
        headers: Vec<(&'static str, &'static str)>,
        body: &'static str,
    },
    Fail(TransportError),
}

#[derive(Clone, Debug)]
pub enum RecordedBody {
    Empty,
    Json(Vec<u8>),
    Bytes {
        content_type: HeaderValue,
        data: Vec<u8>,
    },
    Multipart {
        boundary: String,
        parts: Vec<MultipartPart>,
    },
}

#[derive(Clone, Debug)]
pub struct Call {
    pub method: Method,
    pub url: String,
    pub headers: HeaderMap,
    pub body: RecordedBody,
    pub deadline: Duration,
}

pub struct Script {
    answers: Mutex<VecDeque<Outcome>>,
    calls: Mutex<Vec<Call>>,
}

impl Script {
    pub fn new(answers: Vec<Outcome>) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from(answers)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl RemoteTransport for Script {
    fn call(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.calls.lock().unwrap().push(record(request));
        let outcome = self.answers.lock().unwrap().pop_front().unwrap();
        ready(respond(outcome))
    }
}

fn record(request: OutboundRequest) -> Call {
    let body = match request.body {
        RequestBody::Empty => RecordedBody::Empty,
        RequestBody::Json(data) => RecordedBody::Json(data),
        RequestBody::Bytes { content_type, data } => RecordedBody::Bytes { content_type, data },
        RequestBody::Multipart { boundary, parts } => RecordedBody::Multipart { boundary, parts },
    };
    Call {
        method: request.method,
        url: request.url.to_string(),
        headers: request.headers,
        body,
        deadline: request.deadline,
    }
}

fn respond(outcome: Outcome) -> Result<UpstreamResponse, TransportError> {
    match outcome {
        Outcome::Reply {
            status,
            headers,
            body,
        } => Ok(UpstreamResponse {
            status,
            headers: header_map(&headers),
            body: body.as_bytes().to_vec(),
        }),
        Outcome::Fail(error) => Err(error),
    }
}

fn header_map(headers: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for &(name, value) in headers {
        map.insert(
            HeaderName::from_static(name),
            HeaderValue::from_bytes(value.as_bytes()).unwrap(),
        );
    }
    map
}

#[derive(Default)]
pub struct Clock {
    sleeps: Mutex<Vec<Duration>>,
}

impl Clock {
    pub fn sleeps(&self) -> Vec<Duration> {
        self.sleeps.lock().unwrap().clone()
    }
}

impl Sleeper for Clock {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.sleeps.lock().unwrap().push(duration);
        ready(())
    }
}

pub fn remote<'call>(
    transport: &'call Script,
    sleeper: &'call Clock,
    prepared: &'call Prepared,
) -> SonioxRemote<'call, Script, Clock> {
    SonioxRemote {
        transport,
        sleeper,
        prepared,
        timeout: TIMEOUT,
        limits: ApiLimits::default(),
        boundaries: || BOUNDARY.to_owned(),
    }
}

pub fn prepared(options: HandlerOptions, headers: &[(&str, &str)]) -> Prepared {
    let mut auth_headers = Headers::new();
    for &(name, value) in headers {
        auth_headers.insert(name.to_owned(), value.to_owned());
    }
    Prepared {
        auth_headers,
        api_base: API_BASE.to_owned(),
        params: Object::new(),
        options,
    }
}

pub fn options() -> HandlerOptions {
    HandlerOptions {
        poll_interval: Decimal::ONE,
        max_attempts: 3,
        cleanup: vec!["transcription".to_owned(), "file".to_owned()],
        filename_override: None,
        audio_url: None,
        file_id: None,
        response_format: None,
    }
}

pub fn audio() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: b"RIFF-data".to_vec(),
        filename: "audio.mp3".to_owned(),
        content_type: "audio/mpeg".to_owned(),
    }
}

pub fn auth() -> [(&'static str, &'static str); 2] {
    [("Authorization", "Bearer sk-local"), ("x-client", "suite")]
}
