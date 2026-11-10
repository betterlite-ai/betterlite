use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use tokio::runtime::{Builder, Runtime};

use crate::image_transport::error::ImageTransportError;
use crate::image_transport::json::{JsonMap, JsonValue};
use crate::image_transport::request::{
    Call, CallStyle, Connection, ImageClientConstructor, PollingSettings, TimeoutPhases, execute,
};
use crate::image_transport::{ImageClock, ImageReply, ImageRequest, ImageResponse, ImageTransport};

pub(super) const PREVIEW_VERSION: &str = "2023-06-01-preview";
pub(super) const DIRECT_VERSION: &str = "2024-02-01";
pub(super) const PREVIEW_BASE: &str = "https://example.test/openai/deployments/dall-e-2/images/generations?api-version=2023-06-01-preview";
pub(super) const PREVIEW_QUERY_BASE: &str = "https://example.test/openai/deployments/dall-e-2/images/generations?api-version=2023-06-01-preview&tag=a%20b#keep";
pub(super) const SUBMIT_URL: &str =
    "https://example.test/openai/images/generations:submit?api-version=2023-06-01-preview";
pub(super) const SUBMIT_QUERY_URL: &str = "https://example.test/openai/images/generations:submit?api-version=2023-06-01-preview&tag=a%20b#keep";
pub(super) const OPERATION: &str = "https://example.test/openai/operations/op-1";
pub(super) const DEPLOYMENT_BASE: &str =
    "https://example.test/openai/deployments/dall-e-2/images/generations?api-version=2024-02-01";
pub(super) const V1_BASE: &str =
    "https://example.test/openai/v1/images/generations?api-version=2024-02-01";
pub(super) const PROVIDER_BASE: &str =
    "https://example.test/providers/black-forest-labs/flux%20pro/images/generations";
pub(super) const SYNTHETIC_URL: &str = "https://api.openai.com/v1";
pub(super) const API_KEY: &str = "fixture-secret-key";
pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) fn headers() -> Vec<(&'static str, &'static str)> {
    vec![("api-key", API_KEY), ("content-type", "application/json")]
}

pub(super) fn header_map(headers: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for &(name, value) in headers {
        map.insert(
            HeaderName::from_static(name),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    map
}

pub(super) fn reply(
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static str,
) -> Reply {
    Reply {
        status,
        headers,
        body,
        delay: 0,
    }
}

#[derive(Clone)]
pub(super) struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, &'static str)>,
    pub body: &'static str,
    pub delay: u64,
}

pub(super) struct Step {
    pub api_base: &'static str,
    pub api_version: &'static str,
    pub deployment_name: Option<&'static str>,
    pub data: &'static str,
    pub headers: Vec<(&'static str, &'static str)>,
    pub environment: BTreeMap<String, String>,
    pub style: CallStyle,
    pub supplied_client: bool,
    pub timeout_argument: Option<f64>,
    pub replies: Vec<Reply>,
}

impl Step {
    pub(super) fn preview(replies: Vec<Reply>) -> Self {
        Self {
            api_version: PREVIEW_VERSION,
            ..Self::direct(PREVIEW_BASE, Some("dall-e-2"), replies)
        }
    }

    pub(super) fn direct(
        api_base: &'static str,
        deployment_name: Option<&'static str>,
        replies: Vec<Reply>,
    ) -> Self {
        Self {
            api_base,
            api_version: DIRECT_VERSION,
            deployment_name,
            data: r#"{"model":"dall-e-2","prompt":"a cat","n":1}"#,
            headers: headers(),
            environment: BTreeMap::new(),
            style: CallStyle::Async,
            supplied_client: false,
            timeout_argument: None,
            replies,
        }
    }

    pub(super) fn with_timeout(mut self, seconds: &str) -> Self {
        self.environment.insert(
            "AZURE_OPERATION_POLLING_TIMEOUT".to_owned(),
            seconds.to_owned(),
        );
        self
    }

    pub(super) fn sync(mut self) -> Self {
        self.style = CallStyle::Sync;
        self
    }
}

pub(super) fn polling_timeout(environment: &BTreeMap<String, String>) -> Duration {
    ConfigLoader {
        lookup: environment.clone(),
    }
    .load("")
    .map(|config| {
        config
            .sections
            .azure_image_transport
            .operation_polling_timeout
    })
    .unwrap()
}

pub(super) struct Outcome {
    pub result: Result<ImageResponse, ImageTransportError>,
    pub data: JsonMap,
    pub harness: Harness,
}

pub(super) fn run(step: Step) -> Outcome {
    let harness = Harness::new(step.replies);
    let mut data: JsonMap = serde_json::from_str(step.data).unwrap();
    let connection = Connection {
        client: step.supplied_client.then(|| harness.transport()),
        timeout: step.timeout_argument,
    };
    let settings = PollingSettings {
        timeout: polling_timeout(&step.environment),
        style: step.style,
    };
    let call = Call {
        api_base: step.api_base,
        api_version: step.api_version,
        deployment_name: step.deployment_name,
        data: &mut data,
        headers: header_map(&step.headers),
    };
    let constructor = harness.constructor();
    let result = runtime().block_on(execute(
        &constructor,
        connection,
        &*harness.clock,
        settings,
        call,
    ));
    Outcome {
        result,
        data,
        harness,
    }
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

pub(super) struct Harness {
    state: Arc<FixtureState>,
    constructed: Arc<Mutex<Vec<TimeoutPhases>>>,
    clock: Arc<FixtureClock>,
}

impl Harness {
    fn new(replies: Vec<Reply>) -> Self {
        let clock = Arc::new(FixtureClock {
            elapsed: AtomicU64::new(0),
            sleeps: Mutex::default(),
        });
        Self {
            state: Arc::new(FixtureState {
                replies: Mutex::new(VecDeque::from(replies)),
                calls: Mutex::default(),
                debug: Mutex::default(),
                clock: Arc::clone(&clock),
            }),
            constructed: Arc::default(),
            clock,
        }
    }

    pub(super) fn transport(&self) -> FixtureTransport {
        FixtureTransport {
            state: Arc::clone(&self.state),
        }
    }

    pub(super) fn constructor(&self) -> FixtureConstructor {
        FixtureConstructor {
            transport: self.transport(),
            constructed: Arc::clone(&self.constructed),
        }
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        lock(&self.state.calls).clone()
    }

    pub(super) fn request_debug(&self) -> Vec<String> {
        lock(&self.state.debug).clone()
    }

    pub(super) fn sleeps(&self) -> Vec<i64> {
        lock(&self.clock.sleeps).clone()
    }

    pub(super) fn constructed(&self) -> Vec<TimeoutPhases> {
        lock(&self.constructed).clone()
    }

    pub(super) fn elapsed_seconds(&self) -> u64 {
        self.clock.elapsed.load(Ordering::SeqCst)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct FixtureState {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<RecordedCall>>,
    debug: Mutex<Vec<String>>,
    clock: Arc<FixtureClock>,
}

#[derive(Debug, Clone)]
pub(super) struct RecordedCall {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<JsonValue>,
}

impl RecordedCall {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone)]
pub(super) struct FixtureTransport {
    state: Arc<FixtureState>,
}

impl ImageTransport for FixtureTransport {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<Output = Result<ImageReply, TransportError>> + Send {
        lock(&self.state.debug).push(format!("{request:?}"));
        lock(&self.state.calls).push(RecordedCall {
            method: request.method.to_string(),
            url: request.url.clone(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
            body: (!request.body.is_empty())
                .then(|| serde_json::from_slice(&request.body).unwrap()),
        });
        let reply = lock(&self.state.replies)
            .pop_front()
            .expect("a scripted reply");
        if reply.delay > 0 {
            self.state.clock.advance(reply.delay);
        }
        ready(Ok(ImageReply {
            status: reply.status,
            headers: header_map(&reply.headers),
            body: reply.body.as_bytes().to_vec(),
        }))
    }
}

pub(super) struct FixtureClock {
    elapsed: AtomicU64,
    sleeps: Mutex<Vec<i64>>,
}

impl FixtureClock {
    fn advance(&self, seconds: u64) {
        self.elapsed.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl ImageClock for FixtureClock {
    fn now(&self) -> Duration {
        Duration::from_secs(self.elapsed.load(Ordering::SeqCst))
    }

    fn pause(&self, seconds: i64) -> impl Future<Output = ()> + Send {
        lock(&self.sleeps).push(seconds);
        if let Ok(positive) = u64::try_from(seconds) {
            self.advance(positive);
        }
        ready(())
    }
}

pub(super) struct FixtureConstructor {
    transport: FixtureTransport,
    constructed: Arc<Mutex<Vec<TimeoutPhases>>>,
}

impl ImageClientConstructor for FixtureConstructor {
    type Client = FixtureTransport;

    fn construct(&self, timeout: TimeoutPhases) -> FixtureTransport {
        lock(&self.constructed).push(timeout);
        self.transport.clone()
    }
}

pub(super) fn json(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

pub(super) fn parsed(body: &[u8]) -> JsonValue {
    serde_json::from_slice(body).unwrap()
}

pub(super) fn data_json(data: &JsonMap) -> JsonValue {
    JsonValue::Object(data.clone())
}

pub(super) fn text(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn failure_of(outcome: &Outcome) -> &ImageTransportError {
    outcome.result.as_ref().expect_err("the call fails")
}

pub(super) fn success(outcome: &Outcome) -> &ImageResponse {
    outcome.result.as_ref().expect("the call succeeds")
}

pub(super) fn request_metadata(response: &ImageResponse) -> (String, String) {
    (
        response.request.method.to_string(),
        response.request.url.clone(),
    )
}

pub(super) fn operation_error(error: &ImageTransportError) -> (&JsonValue, &str) {
    match error {
        ImageTransportError::OperationFailed { message, body } => (body, message),
        ImageTransportError::RejectedPollingUrl { .. }
        | ImageTransportError::MissingOperationLocation { .. }
        | ImageTransportError::MissingStatusInitial
        | ImageTransportError::PollingTimeout
        | ImageTransportError::Decode(_)
        | ImageTransportError::MissingStatus
        | ImageTransportError::MissingResult
        | ImageTransportError::RetryAfter { .. }
        | ImageTransportError::NegativeSleep { .. }
        | ImageTransportError::Url(_)
        | ImageTransportError::Encode(_)
        | ImageTransportError::Transport(_) => panic!("expected an operation failure"),
    }
}
