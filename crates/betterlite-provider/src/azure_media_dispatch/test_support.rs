use std::collections::{HashMap, VecDeque};
use std::future::{Future, ready};
use std::mem::take;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{ClientIdentity, ClientKind};
use betterlite_azure::image_transport::request::{ImageClientConstructor, TimeoutPhases};
use betterlite_azure::image_transport::{ImageClock, ImageReply, ImageRequest, ImageTransport};
use betterlite_azure::transcription::call::logging::{
    AzureTranscriptionLogger, AzureTranscriptionPostCall, AzureTranscriptionPreCall,
};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::transcription::transport::TranscriptionTransport;
use http::header::{HeaderMap, HeaderValue};

use super::AzureMediaRequest;
use super::call::AzureMediaOperation;
use super::error::AzureMediaError;
use crate::operation::{OperationRequest, OperationResult};

pub mod exchange;
pub mod speech;

use self::speech::{FixtureError, SpeechLogging};

// one scripted reply per request the transport will receive, in order
pub struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Reply {
    #[must_use]
    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: &'static str, value: &'static str) -> Self {
        self.headers.insert(name, HeaderValue::from_static(value));
        self
    }
}

struct State {
    replies: Mutex<VecDeque<Reply>>,
    images: Mutex<Vec<ImageRequest>>,
    outbound: Mutex<Vec<OutboundRequest>>,
    timeouts: Mutex<Vec<TimeoutPhases>>,
}

// the image and transcription transports share one script, so a test names every wire call in order
#[derive(Clone)]
pub struct Fixture {
    state: Arc<State>,
}

impl Fixture {
    #[must_use]
    pub fn new(replies: Vec<Reply>) -> Self {
        Self {
            state: Arc::new(State {
                replies: Mutex::new(VecDeque::from(replies)),
                images: Mutex::new(Vec::new()),
                outbound: Mutex::new(Vec::new()),
                timeouts: Mutex::new(Vec::new()),
            }),
        }
    }

    #[must_use]
    pub fn images(&self) -> Vec<ImageRequest> {
        match self.state.images.lock() {
            Ok(images) => images.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    #[must_use]
    pub fn outbound(&self) -> Vec<OutboundRequest> {
        match self.state.outbound.lock() {
            Ok(mut outbound) => take(&mut *outbound),
            Err(poisoned) => take(&mut *poisoned.into_inner()),
        }
    }

    #[must_use]
    pub fn timeouts(&self) -> Vec<TimeoutPhases> {
        match self.state.timeouts.lock() {
            Ok(timeouts) => timeouts.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl ImageTransport for Fixture {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        locked(&self.state.images).push(request);
        ready(Ok(image_reply(scripted(&self.state))))
    }
}

impl TranscriptionTransport for Fixture {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        locked(&self.state.outbound).push(request);
        ready(Ok(upstream_reply(scripted(&self.state))))
    }
}

// A test that scripts fewer replies than its dispatch sends sees a 500 instead of a panic.
fn scripted(state: &State) -> Reply {
    locked(&state.replies)
        .pop_front()
        .unwrap_or_else(|| Reply::json(500, "no scripted reply"))
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn image_reply(reply: Reply) -> ImageReply {
    ImageReply {
        status: reply.status,
        headers: reply.headers,
        body: reply.body,
    }
}

fn upstream_reply(reply: Reply) -> UpstreamResponse {
    UpstreamResponse {
        status: reply.status,
        headers: reply.headers,
        body: reply.body,
    }
}

pub struct Clock;

impl ImageClock for Clock {
    fn now(&self) -> Duration {
        Duration::from_secs(1)
    }

    fn pause(&self, _seconds: i64) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

impl ImageClientConstructor for Fixture {
    type Client = Self;

    fn construct(&self, timeout: TimeoutPhases) -> Self {
        locked(&self.state.timeouts).push(timeout);
        self.clone()
    }
}

#[derive(Default)]
pub struct Cache {
    entries: HashMap<ClientIdentity, ClientKind>,
}

impl ClientCache for Cache {
    fn get(&self, identity: &ClientIdentity) -> Option<ClientKind> {
        self.entries.get(identity).copied()
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, _owned: bool) {
        self.entries.insert(identity, kind);
    }
}

#[derive(Default)]
pub struct Recorder {
    pub events: Vec<&'static str>,
}

impl ImageGenerationLogger for Recorder {
    fn pre_call(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        self.events.push("image_pre");
        Ok(())
    }

    fn post_call(&mut self, _record: PostCallRecord) {
        self.events.push("image_post");
    }
}

impl AzureTranscriptionLogger for Recorder {
    fn pre_call(&mut self, _record: AzureTranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        self.events.push("audio_pre");
        Ok(())
    }

    fn post_call(&mut self, _record: AzureTranscriptionPostCall<'_>) {
        self.events.push("audio_post");
    }
}

pub struct Outcome {
    pub result: Result<OperationResult, AzureMediaError>,
    pub fixture: Fixture,
    pub logger: Recorder,
    pub speech: SpeechLogging,
}

// Every Azure media case dispatches the same unit request; only the speech logging fixture can fail to build
pub async fn exchange(
    operation: AzureMediaOperation,
    replies: Vec<Reply>,
) -> Result<Outcome, FixtureError> {
    exchange::exchange_request_with(
        OperationRequest::AzureMedia(AzureMediaRequest),
        operation,
        replies,
        |_logging| {},
    )
    .await
}
