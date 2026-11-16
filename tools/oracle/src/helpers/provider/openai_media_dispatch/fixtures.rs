use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
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
use betterlite_oracle_helper_api::error::HelperError;
use futures_util::StreamExt as _;
use futures_util::stream::iter;
use http::HeaderMap;
use indexmap::IndexMap;
use serde_json::Value;

use super::failure;
use super::views::text;

// The fake upstream every leaf reaches: it records the request it was handed and answers from the fixture.
pub(super) struct Fixture {
    status: u16,
    body: Vec<u8>,
    headers: HeaderMap,
    seen: Mutex<Vec<OutboundRequest>>,
}

impl Fixture {
    pub(super) fn new(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            headers: HeaderMap::new(),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn with_headers(status: u16, body: Vec<u8>, headers: HeaderMap) -> Self {
        Self {
            status,
            body,
            headers,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn only_request(&self) -> Result<OutboundRequest, Box<HelperError>> {
        let mut seen = self
            .seen
            .lock()
            .map_err(|error| failure("Fixture", error))?;
        if seen.len() != 1 {
            return Err(failure(
                "Fixture",
                format!("one dispatch sends one request, found {}", seen.len()),
            ));
        }
        seen.pop()
            .ok_or_else(|| failure("Fixture", "the fixture recorded no request"))
    }

    pub(super) fn request_count(&self) -> usize {
        self.seen.lock().map(|seen| seen.len()).unwrap_or_default()
    }

    fn record(&self, request: OutboundRequest) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(request);
        }
    }

    fn answer(&self) -> UpstreamResponse {
        UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
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
            headers: self.headers.clone(),
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
pub(super) struct Recorder;

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

pub(super) fn header(request: &OutboundRequest, name: &str) -> Option<String> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub(super) fn json_body(request: &OutboundRequest) -> Result<Value, Box<HelperError>> {
    let RequestBody::Json(bytes) = &request.body else {
        return Err(failure("Fixture", "the request body is not JSON"));
    };
    serde_json::from_slice(bytes).map_err(|error| failure("Fixture", error))
}

pub(super) fn multipart_view(request: &OutboundRequest) -> Result<Value, Box<HelperError>> {
    let RequestBody::Multipart { parts, .. } = &request.body else {
        return Err(failure("Fixture", "the request is not multipart"));
    };
    let mut rendered = Vec::with_capacity(parts.len());
    for part in parts {
        rendered.push(serde_json::json!({
            "name": part.name,
            "filename": part.file_name,
            "content_type": part.content_type,
            "body": text(&part.data)?,
        }));
    }
    Ok(Value::Array(rendered))
}
