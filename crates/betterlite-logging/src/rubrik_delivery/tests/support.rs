use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::rubrik_delivery::clock::Clock;
use betterlite_logging::rubrik_delivery::config::RubrikInit;
use betterlite_logging::rubrik_delivery::error::RubrikError;
use betterlite_logging::rubrik_delivery::logger::RubrikLogger;
use betterlite_logging::rubrik_delivery::queue::push_payload;
use betterlite_logging::rubrik_delivery::transport::{
    ClientTier, JsonPost, Timeout, Transport, TransportFailure, TransportResponse,
};
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

pub(super) type Environment = BTreeMap<String, String>;

pub(super) type LoggerPair = (Arc<RubrikLogger>, Arc<ScriptedTransport>);

#[derive(Clone, Copy, Debug)]
pub(super) struct FixedClock(pub(super) Duration);

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        self.0
    }
}

#[derive(Debug, Default)]
pub(super) struct SequenceClock {
    values: Mutex<VecDeque<Duration>>,
}

impl SequenceClock {
    pub(super) fn new(values: &[Duration]) -> Self {
        Self {
            values: Mutex::new(values.iter().copied().collect()),
        }
    }
}

impl Clock for SequenceClock {
    fn now_unix(&self) -> Duration {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub(super) struct RecordedCall {
    pub(super) tier: ClientTier,
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) payload: Json,
    pub(super) timeout: Timeout,
}

impl RecordedCall {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone, Debug)]
pub(super) enum Outcome {
    Reply { status: u16, body: String },
    Network,
}

#[derive(Clone, Debug)]
pub(super) struct ScriptedReply {
    pub(super) outcome: Outcome,
    pub(super) during_send: Option<Json>,
}

impl ScriptedReply {
    pub(super) fn ok(body: &str) -> Self {
        Self {
            outcome: Outcome::Reply {
                status: 200,
                body: body.to_owned(),
            },
            during_send: None,
        }
    }

    pub(super) fn status(status: u16) -> Self {
        Self {
            outcome: Outcome::Reply {
                status,
                body: String::new(),
            },
            during_send: None,
        }
    }

    pub(super) fn network() -> Self {
        Self {
            outcome: Outcome::Network,
            during_send: None,
        }
    }

    pub(super) fn pushing(mut self, payload: Json) -> Self {
        self.during_send = Some(payload);
        self
    }
}

#[derive(Debug, Default)]
pub(super) struct ScriptedTransport {
    replies: Mutex<VecDeque<ScriptedReply>>,
    calls: Mutex<Vec<RecordedCall>>,
    logger: Mutex<Option<Weak<RubrikLogger>>>,
}

impl ScriptedTransport {
    pub(super) fn new(replies: Vec<ScriptedReply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(Vec::new()),
            logger: Mutex::new(None),
        })
    }

    pub(super) fn attach(&self, logger: &Arc<RubrikLogger>) {
        let mut slot = self.logger.lock().unwrap_or_else(PoisonError::into_inner);
        *slot = Some(Arc::downgrade(logger));
    }

    pub(super) fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn call_count(&self) -> usize {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

fn record(transport: &ScriptedTransport, request: JsonPost) {
    let payload = serde_json::from_slice::<Json>(&request.body).expect("recorded body is JSON");
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_ascii_lowercase(),
                value.to_str().expect("header value is text").to_owned(),
            )
        })
        .collect();
    transport
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(RecordedCall {
            tier: request.tier,
            url: request.url,
            headers,
            payload,
            timeout: request.timeout,
        });
}

fn next_reply(transport: &ScriptedTransport) -> ScriptedReply {
    transport
        .replies
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .pop_front()
        .expect("the scripted transport has a reply for every request")
}

fn push_during_send(transport: &ScriptedTransport, reply: &ScriptedReply) {
    let Some(queued) = reply.during_send.clone() else {
        return;
    };
    let logger = transport
        .logger
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(Weak::upgrade)
        .expect("the scripted transport is attached to the logger");
    push_payload(&logger, queued);
}

#[async_trait]
impl Transport for ScriptedTransport {
    async fn post_json(&self, request: JsonPost) -> Result<TransportResponse, TransportFailure> {
        record(self, request);
        let reply = next_reply(self);
        push_during_send(self, &reply);
        match reply.outcome {
            Outcome::Reply { status, body } => Ok(TransportResponse {
                status,
                body: body.into_bytes(),
            }),
            Outcome::Network => Err(scripted_failure()),
        }
    }
}

// A reqwest error cannot be built in a test, so the scripted network failure reports a timeout through the typed HTTP error.
fn scripted_failure() -> TransportFailure {
    TransportFailure::Request {
        source: TransportError::Rejected(HttpError::Timeout),
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn init(api_base: &str, api_key: &str) -> RubrikInit {
    RubrikInit {
        api_key: Some(SecretString::from(api_key)),
        api_base: Some(api_base.to_owned()),
        event_hook: None,
        default_on: None,
        guardrail_name: None,
        timeout: None,
    }
}

pub(super) fn bare_init() -> RubrikInit {
    RubrikInit {
        api_key: None,
        api_base: None,
        event_hook: None,
        default_on: None,
        guardrail_name: None,
        timeout: None,
    }
}

pub(super) fn build(
    init: &RubrikInit,
    environment: &Environment,
    clock: Arc<dyn Clock>,
    replies: Vec<ScriptedReply>,
) -> Result<LoggerPair, RubrikError> {
    let transport = ScriptedTransport::new(replies);
    let built = RubrikLogger::new(
        init,
        environment,
        Arc::<ScriptedTransport>::clone(&transport),
        clock,
    )?;
    let logger = Arc::new(built);
    transport.attach(&logger);
    Ok((logger, transport))
}

pub(super) fn logger(init: &RubrikInit, replies: Vec<ScriptedReply>) -> LoggerPair {
    logger_with_environment(init, &Environment::new(), replies)
}

pub(super) fn logger_with_environment(
    init: &RubrikInit,
    environment: &Environment,
    replies: Vec<ScriptedReply>,
) -> LoggerPair {
    build(
        init,
        environment,
        Arc::new(FixedClock(Duration::from_secs(1))),
        replies,
    )
    .expect("the logger initializes")
}

pub(super) fn logger_with_clock(
    init: &RubrikInit,
    clock: SequenceClock,
    replies: Vec<ScriptedReply>,
) -> LoggerPair {
    build(init, &Environment::new(), Arc::new(clock), replies).expect("the logger initializes")
}

pub(super) fn payload(text: &str) -> Json {
    serde_json::from_str::<Json>(text).expect("payload text is JSON")
}

pub(super) fn object(entries: &[(&str, Json)]) -> Json {
    Json::Object(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    )
}

pub(super) fn number(value: f64) -> Json {
    Json::Number(serde_json::Number::from_f64(value).expect("a finite test number"))
}

pub(super) fn integer(value: u64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

pub(super) fn empty_policy() -> Arc<ProxyPolicy> {
    Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("an empty proxy policy resolves"),
    )
}

pub(super) fn moderation_timeout() -> Timeout {
    Timeout {
        total: Duration::from_secs(5),
        connect: Duration::from_secs(2),
    }
}

pub(super) fn logging_timeout() -> Timeout {
    Timeout {
        total: Duration::from_secs(600),
        connect: Duration::from_secs(5),
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedWarnings(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl CapturedWarnings {
    pub(super) fn count_containing(&self, needle: &str) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|message| message.contains(needle))
            .count()
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedWarnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture<Action, Output>(action: Action) -> (Output, CapturedWarnings)
where
    Action: FnOnce() -> Output,
{
    let warnings = CapturedWarnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let output = with_default(subscriber, action);
    (output, warnings)
}

pub(super) fn fill(logger: &RubrikLogger, start: u64, count: u64) {
    for index in start..start.saturating_add(count) {
        push_payload(logger, object(&[("index", integer(index))]));
    }
}

pub(super) fn index_of(payload: &Json) -> Option<u64> {
    let Json::Object(fields) = payload else {
        return None;
    };
    match fields.get("index") {
        Some(Json::Number(number)) => number.as_u64(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
