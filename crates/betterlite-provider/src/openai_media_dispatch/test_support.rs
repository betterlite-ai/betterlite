use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
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
use futures_util::StreamExt as _;
use futures_util::stream::iter;
use http::HeaderMap;
use indexmap::IndexMap;

// The fake upstream every media leaf reaches: it records the request it was handed and answers from the fixture.
pub struct Fixture {
    status: u16,
    body: Vec<u8>,
    headers: HeaderMap,
    seen: Mutex<Vec<OutboundRequest>>,
}

impl Fixture {
    #[must_use]
    pub fn new(status: u16, body: &[u8]) -> Self {
        Self {
            status,
            body: body.to_vec(),
            headers: HeaderMap::new(),
            seen: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn with_headers(status: u16, body: &[u8], headers: HeaderMap) -> Self {
        Self {
            status,
            body: body.to_vec(),
            headers,
            seen: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn request_count(&self) -> usize {
        let Ok(seen) = self.seen.lock() else {
            return 0;
        };
        seen.len()
    }

    pub fn take_request(&self) -> Option<OutboundRequest> {
        match self.seen.lock() {
            Ok(mut seen) if seen.len() == 1 => seen.pop(),
            Ok(_) | Err(_) => None,
        }
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
pub struct Recorder;

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
