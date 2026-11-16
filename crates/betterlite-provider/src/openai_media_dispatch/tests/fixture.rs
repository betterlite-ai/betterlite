use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::files::transport::StreamReply;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_generation::transport::ImageGenerationTransport;
use betterlite_openai::image_variations::logging::{
    ImageVariationLogger, PostCallRecord as VariationPostCall, PreCallRecord as VariationPreCall,
};
use betterlite_openai::image_variations::transport::ImageVariationTransport;
use betterlite_openai::speech::logging::{SpeechLogRecord, SpeechLogger};
use betterlite_openai::speech::result::SpeechTransport;
use betterlite_openai::transcription::logging::{
    TranscriptionLogger, TranscriptionPostCall, TranscriptionPreCall,
};
use betterlite_openai::transcription::transport::TranscriptionTransport;
use betterlite_openai::videos::VideoOptions;
use futures_util::StreamExt as _;
use futures_util::stream::iter;
use http::HeaderMap;
use indexmap::IndexMap;

use crate::openai_media_dispatch::OpenAiMediaResult;
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::operation::OperationResult;

// The fake upstream every media leaf reaches: it records the request it was handed and answers from the fixture.
pub(crate) struct Fixture {
    status: u16,
    body: Vec<u8>,
    seen: Mutex<Vec<OutboundRequest>>,
}

impl Fixture {
    pub(crate) fn new(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            body: body.to_vec(),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn only_request(&self) -> OutboundRequest {
        let mut seen = self.seen.lock().expect("the fixture lock is free");
        assert_eq!(seen.len(), 1, "one dispatch sends one request");
        seen.remove(0)
    }

    pub(crate) fn request_count(&self) -> usize {
        self.seen.lock().expect("the fixture lock is free").len()
    }

    fn record(&self, request: OutboundRequest) {
        self.seen
            .lock()
            .expect("the fixture lock is free")
            .push(request);
    }

    fn answer(&self) -> UpstreamResponse {
        UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone(),
        }
    }
}

impl ImageGenerationTransport for Fixture {
    fn generate(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        ready(Ok(self.answer()))
    }
}

impl ImageVariationTransport for Fixture {
    fn create_variation(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        ready(Ok(self.answer()))
    }
}

impl SpeechTransport for Fixture {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send {
        self.record(request);
        let chunks: Vec<Result<Vec<u8>, HttpError>> = vec![Ok(self.body.clone())];
        ready(Ok(StreamReply {
            status: self.status,
            headers: HeaderMap::new(),
            body: iter(chunks).boxed(),
        }))
    }
}

impl TranscriptionTransport for Fixture {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.record(request);
        ready(Ok(self.answer()))
    }
}

// A logger that keeps nothing: the composition must pass every record through and stay silent.
pub(crate) struct Recorder;

impl ImageGenerationLogger for Recorder {
    fn pre_call(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        Ok(())
    }

    fn post_call(&mut self, _record: PostCallRecord) {}
}

impl ImageVariationLogger for Recorder {
    fn pre_call(&mut self, _record: VariationPreCall) -> Result<(), PrePostError> {
        Ok(())
    }

    fn post_call(&mut self, _record: VariationPostCall) {}
}

impl SpeechLogger for Recorder {
    fn pre_call(&mut self, _record: SpeechLogRecord<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}

impl TranscriptionLogger for Recorder {
    fn pre_call(&mut self, _record: TranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        Ok(())
    }

    fn response_headers(&mut self, _headers: &IndexMap<String, String>) {}

    fn post_call(&mut self, _record: TranscriptionPostCall<'_>) {}
}

pub(crate) fn media_result(result: Result<OperationResult, OpenAiMediaError>) -> OpenAiMediaResult {
    match result {
        Ok(OperationResult::OpenAiMedia(result)) => result,
        Ok(_) => panic!("the media dispatcher returns a media result"),
        Err(error) => panic!("the media dispatch failed: {error}"),
    }
}

pub(crate) fn media_error(result: Result<OperationResult, OpenAiMediaError>) -> OpenAiMediaError {
    match result {
        Ok(_) => panic!("the dispatch should fail"),
        Err(error) => error,
    }
}

pub(crate) fn json_body(request: &OutboundRequest) -> IndexMap<String, Json> {
    serde_json::from_slice(json_bytes(request)).expect("the request body is a JSON object")
}

pub(crate) fn parts(request: &OutboundRequest) -> &[MultipartPart] {
    let RequestBody::Multipart { parts, .. } = &request.body else {
        panic!("the request body is multipart");
    };
    parts
}

pub(crate) fn boundary(request: &OutboundRequest) -> &str {
    let RequestBody::Multipart { boundary, .. } = &request.body else {
        panic!("the request body is multipart");
    };
    boundary
}

pub(crate) fn options(source: &str) -> IndexMap<String, Json> {
    serde_json::from_str(source).expect("the options are a JSON object")
}

pub(crate) fn video_options(source: &str) -> VideoOptions {
    serde_json::from_str(source).expect("the options are a JSON object")
}

fn json_bytes(request: &OutboundRequest) -> &[u8] {
    let RequestBody::Json(bytes) = &request.body else {
        panic!("the request body is JSON");
    };
    bytes
}
