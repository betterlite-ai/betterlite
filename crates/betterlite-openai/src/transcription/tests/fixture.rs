use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::audio_file_input::{AudioFile, AudioTuple, ProcessedAudioFile, TupleContent};
use crate::transcription::client::{SharedSession, TranscriptionConnection};
use crate::transcription::logging::{
    TranscriptionLogger, TranscriptionPostCall, TranscriptionPreCall,
};
use crate::transcription::transport::TranscriptionTransport;

#[derive(Debug, Clone, Default)]
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn json(body: &str) -> Self {
        Self::with_headers("application/json", body, true)
    }

    pub(super) fn text(body: &str) -> Self {
        Self::with_headers("text/plain", body, false)
    }

    fn with_headers(content_type: &str, body: &str, request_id: bool) -> Self {
        let mut headers = vec![
            (String::from("content-type"), String::from(content_type)),
            (String::from("content-length"), body.len().to_string()),
        ];
        if request_id {
            headers.push((String::from("x-request-id"), String::from("req-1")));
        }
        Self {
            status: 200,
            headers,
            body: body.as_bytes().to_vec(),
        }
    }
}

// One recorded upstream answer; the request the production code sent is kept for the assertions
pub(super) struct Fixture {
    reply: Reply,
    headers: HeaderMap,
    seen: Mutex<Option<OutboundRequest>>,
    sends: AtomicUsize,
}

impl Fixture {
    pub(super) fn new(reply: Reply) -> Self {
        let mut headers = HeaderMap::new();
        for (name, value) in &reply.headers {
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).expect("the fixture header name is valid"),
                HeaderValue::from_str(value).expect("the fixture header value is valid"),
            );
        }
        Self {
            reply,
            headers,
            seen: Mutex::new(None),
            sends: AtomicUsize::new(0),
        }
    }

    pub(super) fn request(&self) -> OutboundRequest {
        self.seen
            .lock()
            .expect("the request lock is not poisoned")
            .take()
            .expect("the call reached the transport")
    }

    pub(super) fn sends(&self) -> usize {
        self.sends.load(Ordering::Relaxed)
    }
}

impl TranscriptionTransport for Fixture {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        *self.seen.lock().expect("the request lock is not poisoned") = Some(request);
        let _previous = self.sends.fetch_add(1, Ordering::Relaxed);
        ready(Ok(UpstreamResponse {
            status: self.reply.status,
            headers: self.headers.clone(),
            body: self.reply.body.clone(),
        }))
    }
}

pub(super) fn authorization(fixture: &Fixture) -> Option<String> {
    fixture
        .request()
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[derive(Debug, Default)]
pub(super) struct Recorder {
    pub(super) pre_calls: Vec<PreRecord>,
    pub(super) post_calls: Vec<PostRecord>,
    pub(super) response_headers: Vec<IndexMap<String, String>>,
}

#[derive(Debug)]
pub(super) struct PreRecord {
    pub(super) api_base: String,
    pub(super) data: IndexMap<String, Json>,
}

#[derive(Debug)]
pub(super) struct PostRecord {
    pub(super) input: String,
    pub(super) original_response: Json,
    pub(super) data: Option<IndexMap<String, Json>>,
}

impl TranscriptionLogger for Recorder {
    fn pre_call(&mut self, record: TranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        self.pre_calls.push(PreRecord {
            api_base: record.api_base.to_owned(),
            data: record.data,
        });
        Ok(())
    }

    fn response_headers(&mut self, headers: &IndexMap<String, String>) {
        self.response_headers.push(headers.clone());
    }

    fn post_call(&mut self, record: TranscriptionPostCall<'_>) {
        self.post_calls.push(PostRecord {
            input: record.input.to_owned(),
            original_response: record.original_response,
            data: record.data,
        });
    }
}

pub(super) struct AudioFixture {
    pub(super) filename: String,
    pub(super) content: Vec<u8>,
    pub(super) content_type: String,
}

impl AudioFixture {
    pub(super) fn new(filename: &str, content: &[u8], content_type: &str) -> Self {
        Self {
            filename: filename.to_owned(),
            content: content.to_vec(),
            content_type: content_type.to_owned(),
        }
    }

    pub(super) fn processed(&self) -> ProcessedAudioFile {
        ProcessedAudioFile {
            file_content: self.content.clone(),
            filename: self.filename.clone(),
            content_type: self.content_type.clone(),
        }
    }

    pub(super) fn audio_file(&self) -> AudioFile<'_> {
        AudioFile::Tuple(AudioTuple {
            filename: &self.filename,
            content: Some(TupleContent::Bytes(&self.content)),
        })
    }
}

pub(super) struct ConnectionOptions {
    pub(super) api_key: Option<&'static str>,
    pub(super) environment_api_key: Option<&'static str>,
    pub(super) shared_session: bool,
}

pub(super) fn connection(options: &ConnectionOptions) -> TranscriptionConnection {
    TranscriptionConnection {
        api_key: options.api_key.map(SecretString::from),
        environment_api_key: options.environment_api_key.map(SecretString::from),
        api_base: Some(String::from("https://transcription.test/v1")),
        timeout: Duration::from_secs(30),
        max_retries: 2,
        shared_session: options.shared_session.then_some(SharedSession),
    }
}
