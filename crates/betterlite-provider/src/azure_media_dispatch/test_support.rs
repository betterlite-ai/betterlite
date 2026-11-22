pub mod no_sources;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::{Future, ready};
use std::mem::take;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{ClientIdentity, ClientKind};
use betterlite_azure::image_transport::request::{ImageClientConstructor, TimeoutPhases};
use betterlite_azure::image_transport::{ImageClock, ImageReply, ImageRequest, ImageTransport};
use betterlite_azure::transcription::call::logging::{
    AzureTranscriptionLogger, AzureTranscriptionPostCall, AzureTranscriptionPreCall,
};
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_config::sections::azure_url_policy::AzureCredentials;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::transcription::transport::TranscriptionTransport;
use http::header::{HeaderMap, HeaderName, HeaderValue};

use self::no_sources::NoSources;
use super::AzureMediaRequest;
use super::call::AzureMediaOperation;
use super::error::AzureMediaError;
use super::execute::dispatch;
use super::runtime::MediaRuntime;
use crate::operation::{OperationRequest, OperationResult};

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
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        let name =
            HeaderName::from_bytes(name.as_bytes()).expect("a scripted header name is valid");
        let value = HeaderValue::from_str(value).expect("a scripted header value is valid");
        self.headers.insert(name, value);
        self
    }
}

struct State {
    replies: Mutex<VecDeque<Reply>>,
    images: Mutex<Vec<ImageRequest>>,
    outbound: Mutex<Vec<OutboundRequest>>,
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
            }),
        }
    }

    #[must_use]
    pub fn images(&self) -> Vec<ImageRequest> {
        self.state
            .images
            .lock()
            .expect("the image log is not poisoned")
            .clone()
    }

    #[must_use]
    pub fn outbound(&self) -> Vec<OutboundRequest> {
        match self.state.outbound.lock() {
            Ok(mut outbound) => take(&mut *outbound),
            Err(poisoned) => take(&mut *poisoned.into_inner()),
        }
    }
}

impl ImageTransport for Fixture {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        record(&self.state.images, request);
        ready(Ok(image_reply(scripted(&self.state))))
    }
}

impl TranscriptionTransport for Fixture {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        record(&self.state.outbound, request);
        ready(Ok(upstream_reply(scripted(&self.state))))
    }
}

fn record<T>(mutex: &Mutex<Vec<T>>, item: T) {
    mutex
        .lock()
        .expect("the request log is not poisoned")
        .push(item);
}

fn scripted(state: &State) -> Reply {
    state
        .replies
        .lock()
        .expect("the script lock is not poisoned")
        .pop_front()
        .expect("every scripted request has a reply")
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
    type Client = Fixture;

    fn construct(&self, _timeout: TimeoutPhases) -> Fixture {
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
}

pub async fn exchange(operation: AzureMediaOperation, replies: Vec<Reply>) -> Outcome {
    exchange_request(
        OperationRequest::AzureMedia(AzureMediaRequest),
        operation,
        replies,
    )
    .await
}

pub async fn exchange_request(
    request: OperationRequest,
    operation: AzureMediaOperation,
    replies: Vec<Reply>,
) -> Outcome {
    let fixture = Fixture::new(replies);
    let clock = Clock;
    let sources = NoSources;
    let endpoints = EndpointSettings::default();
    let credentials = AzureCredentials::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let mut cache = Cache::default();
    let mut logger = Recorder::default();
    let result = {
        let mut runtime = MediaRuntime {
            endpoints: &endpoints,
            credentials: &credentials,
            environment: &environment,
            sources: &sources,
            refresh_enabled: false,
            polling_timeout: Duration::from_secs(600),
            constructor: &fixture,
            clock: &clock,
            transport: &fixture,
            cache: &mut cache,
            logger: &mut logger,
        };
        dispatch(request, operation, &mut runtime).await
    };
    Outcome {
        result,
        fixture,
        logger,
    }
}
