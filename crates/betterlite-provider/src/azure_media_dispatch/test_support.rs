pub mod no_sources;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::{Future, ready};
use std::mem::take;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
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
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};

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
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    #[must_use]
    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

#[derive(Default)]
struct State {
    replies: Mutex<VecDeque<Reply>>,
    images: Mutex<Vec<ImageRequest>>,
    outbound: Mutex<Vec<OutboundRequest>>,
}

// the image and transcription transports share one script, so a test names every wire call in order
#[derive(Clone, Default)]
pub struct Fixture {
    state: Arc<State>,
}

impl Fixture {
    #[must_use]
    pub fn new(replies: Vec<Reply>) -> Self {
        let fixture = Self::default();
        *lock(&fixture.state.replies) = VecDeque::from(replies);
        fixture
    }

    #[must_use]
    pub fn images(&self) -> Vec<ImageRequest> {
        lock(&self.state.images).clone()
    }

    #[must_use]
    pub fn outbound(&self) -> Vec<OutboundRequest> {
        let mut seen = lock(&self.state.outbound);
        take(&mut *seen)
    }
}

impl ImageTransport for Fixture {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        lock(&self.state.images).push(request);
        let reply = scripted(&self.state);
        ready(Ok(ImageReply {
            status: reply.status,
            headers: headers(&reply.headers),
            body: reply.body,
        }))
    }
}

impl TranscriptionTransport for Fixture {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        lock(&self.state.outbound).push(request);
        let reply = scripted(&self.state);
        ready(Ok(UpstreamResponse {
            status: reply.status,
            headers: headers(&reply.headers),
            body: reply.body,
        }))
    }
}

fn scripted(state: &State) -> Reply {
    lock(&state.replies)
        .pop_front()
        .expect("every scripted request has a reply")
}

fn headers(pairs: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(value) else {
            continue;
        };
        map.insert(name, value);
    }
    map
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

pub struct Constructor {
    pub fixture: Fixture,
}

impl ImageClientConstructor for Constructor {
    type Client = Fixture;

    fn construct(&self, _timeout: TimeoutPhases) -> Fixture {
        self.fixture.clone()
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
    pub image_pre: usize,
    pub image_post: usize,
    pub audio_pre: usize,
    pub audio_post: usize,
}

impl ImageGenerationLogger for Recorder {
    fn pre_call(&mut self, _record: PreCallRecord) -> Result<(), PrePostError> {
        self.image_pre = self.image_pre.saturating_add(1);
        Ok(())
    }

    fn post_call(&mut self, _record: PostCallRecord) {
        self.image_post = self.image_post.saturating_add(1);
    }
}

impl AzureTranscriptionLogger for Recorder {
    fn pre_call(&mut self, _record: AzureTranscriptionPreCall<'_>) -> Result<(), PrePostError> {
        self.audio_pre = self.audio_pre.saturating_add(1);
        Ok(())
    }

    fn post_call(&mut self, _record: AzureTranscriptionPostCall<'_>) {
        self.audio_post = self.audio_post.saturating_add(1);
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
    let constructor = Constructor {
        fixture: fixture.clone(),
    };
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
            constructor: &constructor,
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
