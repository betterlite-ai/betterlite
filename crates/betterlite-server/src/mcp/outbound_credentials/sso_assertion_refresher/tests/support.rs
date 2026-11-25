use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_auth::jwt_claims::TypedValue;
use betterlite_config::sections::settings::JsonValue;
use betterlite_mcp::credential_contracts::token_endpoint_auth::TokenEndpointAuthMethod;
use betterlite_mcp::sso_assertions::assertion::{SsoIdentityAssertion, assertion_from_sso_login};
use betterlite_mcp::sso_assertions::read::{AssertionStoreUnavailable, SsoAssertionStore};
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::Serialize;
use tokio::task::yield_now;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use super::super::client_config::{SsoClientConfig, SsoClientSource};
use super::super::failure::{RefreshFailure, RefreshFailureKind};
use super::super::refresher::{AssertionAccess, AssertionRefresher};
use super::super::store::{
    CoordinatorFactory, DEFAULT_EXPIRY_SKEW, RefreshingAssertionStore, RenewalCoordinator,
    RenewalUnavailable,
};
use super::super::transport::{RefreshResponseBody, RefreshTransport};
use crate::mcp::outbound_credentials::refresh_store::InProcessRefreshCoordinator;
use crate::mcp::outbound_credentials::{RefreshCoordinator, RefreshStep, TokenKey, WallClock};

pub(crate) const TOKEN_ENDPOINT: &str = "https://idp.example/token";
pub(crate) const CLIENT_ID: &str = "client-id";
pub(crate) const CLIENT_SECRET: &str = "client-secret";
pub(crate) const NOW_SECONDS: i64 = 1_700_000_000;
pub(crate) const NOW_MS: i64 = NOW_SECONDS * 1000;

#[derive(Serialize)]
struct Claims {
    iss: &'static str,
    sub: &'static str,
    exp: i64,
}

pub(crate) fn claims(exp: i64) -> JsonValue {
    serde_json::to_value(Claims {
        iss: "https://idp.example",
        sub: "user-1",
        exp,
    })
    .expect("the claims serialize")
}

pub(crate) fn id_token(claims: &JsonValue) -> String {
    let payload = serde_json::to_string(claims).expect("the claims serialize");
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let body = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    format!("{header}.{body}.")
}

pub(crate) fn assertion(exp: i64, refresh_token: Option<&str>) -> SsoIdentityAssertion {
    let id_field = TypedValue {
        json: JsonValue::String(id_token(&claims(exp))),
    };
    let refresh_field = refresh_token.map(|text| TypedValue {
        json: JsonValue::String(text.to_owned()),
    });
    assertion_from_sso_login(Some(&id_field), refresh_field.as_ref())
        .expect("the assertion decodes")
}

pub(crate) fn renewed_body(exp: i64, refresh_token: Option<&str>) -> RefreshResponseBody {
    let mut pairs = vec![("id_token", JsonValue::String(id_token(&claims(exp))))];
    if let Some(text) = refresh_token {
        pairs.push(("refresh_token", JsonValue::String(text.to_owned())));
    }
    body(pairs)
}

pub(crate) fn body(pairs: Vec<(&str, JsonValue)>) -> RefreshResponseBody {
    pairs
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

pub(crate) fn unavailable(message: &str) -> AssertionStoreUnavailable {
    AssertionStoreUnavailable {
        cause: Box::new(io::Error::other(message.to_owned())),
    }
}

pub(crate) fn rejected(detail: &str) -> RefreshFailure {
    RefreshFailure {
        kind: RefreshFailureKind::Rejected,
        detail: detail.to_owned(),
    }
}

pub(crate) fn outage(detail: &str) -> RefreshFailure {
    RefreshFailure {
        kind: RefreshFailureKind::Unavailable,
        detail: detail.to_owned(),
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum AccessFailure {
    None,
    Read,
    Write,
}

#[derive(Clone)]
pub(crate) struct MemoryAccess {
    pub(crate) store: ScriptedInner,
    pub(crate) writes: Arc<Mutex<Vec<(String, SsoIdentityAssertion)>>>,
    pub(crate) failure: AccessFailure,
}

#[async_trait]
impl AssertionAccess for MemoryAccess {
    async fn read(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable> {
        match self.failure {
            AccessFailure::Read => Err(unavailable("the scripted row read failed")),
            AccessFailure::None | AccessFailure::Write => self.store.fetch(user_id).await,
        }
    }

    async fn write(
        &self,
        user_id: &str,
        assertion: &SsoIdentityAssertion,
    ) -> Result<(), AssertionStoreUnavailable> {
        if matches!(self.failure, AccessFailure::Write) {
            return Err(unavailable("the scripted row write failed"));
        }
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((user_id.to_owned(), assertion.clone()));
        self.store
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user_id.to_owned(), assertion.clone());
        Ok(())
    }
}

pub(crate) fn access_with(
    rows: Vec<(&str, SsoIdentityAssertion)>,
    failure: AccessFailure,
) -> MemoryAccess {
    MemoryAccess {
        store: inner(rows),
        writes: Arc::new(Mutex::new(Vec::new())),
        failure,
    }
}

#[derive(Clone)]
pub(crate) struct ScriptedInner {
    pub(crate) rows: Arc<Mutex<HashMap<String, SsoIdentityAssertion>>>,
    pub(crate) fetches: Arc<AtomicUsize>,
    pub(crate) fails: bool,
}

impl SsoAssertionStore for ScriptedInner {
    async fn fetch(
        &self,
        user_id: &str,
    ) -> Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable> {
        yield_now().await;
        self.fetches.fetch_add(1, Ordering::Relaxed);
        if self.fails {
            return Err(unavailable("the scripted inner store failed"));
        }
        Ok(self
            .rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(user_id)
            .cloned())
    }
}

pub(crate) fn inner(rows: Vec<(&str, SsoIdentityAssertion)>) -> ScriptedInner {
    ScriptedInner {
        rows: Arc::new(Mutex::new(
            rows.into_iter()
                .map(|(user, found)| (user.to_owned(), found))
                .collect(),
        )),
        fetches: Arc::new(AtomicUsize::new(0)),
        fails: false,
    }
}

pub(crate) fn failing_inner() -> ScriptedInner {
    ScriptedInner {
        rows: Arc::new(Mutex::new(HashMap::new())),
        fetches: Arc::new(AtomicUsize::new(0)),
        fails: true,
    }
}

pub(crate) struct ClientSpec {
    pub(crate) auth_method: TokenEndpointAuthMethod,
}

pub(crate) struct ScriptedClient {
    pub(crate) spec: Option<ClientSpec>,
}

impl SsoClientSource for ScriptedClient {
    fn client(&self) -> Option<SsoClientConfig> {
        let spec = self.spec.as_ref()?;
        Some(SsoClientConfig {
            token_endpoint: TOKEN_ENDPOINT.to_owned(),
            client_id: CLIENT_ID.to_owned(),
            client_secret: SecretString::from(CLIENT_SECRET.to_owned()),
            auth_method: spec.auth_method,
        })
    }
}

pub(crate) fn basic_client() -> ScriptedClient {
    ScriptedClient {
        spec: Some(ClientSpec {
            auth_method: TokenEndpointAuthMethod::ClientSecretBasic,
        }),
    }
}

pub(crate) fn post_client() -> ScriptedClient {
    ScriptedClient {
        spec: Some(ClientSpec {
            auth_method: TokenEndpointAuthMethod::ClientSecretPost,
        }),
    }
}

pub(crate) fn unconfigured_client() -> ScriptedClient {
    ScriptedClient { spec: None }
}

#[derive(Clone)]
pub(crate) struct RecordedPost {
    pub(crate) url: String,
    pub(crate) form: BTreeMap<String, String>,
    pub(crate) headers: BTreeMap<String, String>,
}

#[derive(Clone)]
pub(crate) struct ScriptedTransport {
    pub(crate) outcomes: Arc<Mutex<VecDeque<Result<RefreshResponseBody, RefreshFailure>>>>,
    pub(crate) calls: Arc<Mutex<Vec<RecordedPost>>>,
    pub(crate) yields: usize,
}

impl ScriptedTransport {
    pub(crate) fn sent(&self) -> Vec<RecordedPost> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RefreshTransport for ScriptedTransport {
    async fn post(
        &self,
        url: &str,
        form: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
    ) -> Result<RefreshResponseBody, RefreshFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedPost {
                url: url.to_owned(),
                form: form.clone(),
                headers: headers.clone(),
            });
        for _ in 0..self.yields {
            yield_now().await;
        }
        self.outcomes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("the case scripts an outcome")
    }
}

pub(crate) fn transport(
    outcomes: Vec<Result<RefreshResponseBody, RefreshFailure>>,
) -> ScriptedTransport {
    ScriptedTransport {
        outcomes: Arc::new(Mutex::new(VecDeque::from(outcomes))),
        calls: Arc::new(Mutex::new(Vec::new())),
        yields: 0,
    }
}

pub(crate) fn yielding_transport(
    outcome: Result<RefreshResponseBody, RefreshFailure>,
) -> ScriptedTransport {
    let mut scripted = transport(vec![outcome]);
    scripted.yields = 3;
    scripted
}

pub(crate) struct ManualClock(pub(crate) Mutex<DateTime<Utc>>);

impl WallClock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

pub(crate) fn clock(now_ms: i64) -> Arc<ManualClock> {
    Arc::new(ManualClock(Mutex::new(
        DateTime::from_timestamp_millis(now_ms).expect("a valid instant"),
    )))
}

#[derive(Clone)]
pub(crate) struct RecordingFactory {
    pub(crate) answers: Arc<Mutex<VecDeque<Option<RenewalCoordinator>>>>,
    pub(crate) asked: Arc<AtomicUsize>,
}

impl CoordinatorFactory for RecordingFactory {
    fn coordinator(&self) -> Option<RenewalCoordinator> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        self.answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default()
    }
}

pub(crate) fn factory(answers: Vec<Option<RenewalCoordinator>>) -> RecordingFactory {
    RecordingFactory {
        answers: Arc::new(Mutex::new(VecDeque::from(answers))),
        asked: Arc::new(AtomicUsize::new(0)),
    }
}

pub(crate) struct RecordingCoordinator {
    pub(crate) runs: Arc<AtomicUsize>,
    pub(crate) losing: bool,
}

#[async_trait]
impl RefreshCoordinator for RecordingCoordinator {
    type Output = ();
    type Error = RenewalUnavailable;

    async fn run(
        &self,
        _key: TokenKey,
        refresh: RefreshStep<'_, (), RenewalUnavailable>,
        reread: RefreshStep<'_, (), RenewalUnavailable>,
    ) -> Result<(), RenewalUnavailable> {
        self.runs.fetch_add(1, Ordering::Relaxed);
        if self.losing {
            return reread().await;
        }
        refresh().await
    }
}

pub(crate) struct RefresherFixture {
    pub(crate) refresher: AssertionRefresher<ScriptedTransport, ScriptedClient, MemoryAccess>,
    pub(crate) transport: ScriptedTransport,
    pub(crate) access: MemoryAccess,
}

impl RefresherFixture {
    pub(crate) fn new(
        transport: ScriptedTransport,
        access: MemoryAccess,
        client: ScriptedClient,
    ) -> Self {
        Self {
            refresher: AssertionRefresher {
                transport: transport.clone(),
                client_source: client,
                access: Arc::new(access.clone()),
            },
            transport,
            access,
        }
    }
}

pub(crate) type FixtureStore = RefreshingAssertionStore<
    ScriptedInner,
    ScriptedTransport,
    ScriptedClient,
    MemoryAccess,
    RecordingFactory,
>;

pub(crate) struct StoreFixture {
    pub(crate) store: FixtureStore,
    pub(crate) inner: ScriptedInner,
    pub(crate) transport: ScriptedTransport,
    pub(crate) access: MemoryAccess,
    pub(crate) factory: RecordingFactory,
}

impl StoreFixture {
    pub(crate) fn new(
        inner: ScriptedInner,
        transport: ScriptedTransport,
        access: MemoryAccess,
        factory: RecordingFactory,
        now_ms: i64,
    ) -> Self {
        Self {
            store: RefreshingAssertionStore {
                inner: inner.clone(),
                refresher: AssertionRefresher {
                    transport: transport.clone(),
                    client_source: basic_client(),
                    access: Arc::new(access.clone()),
                },
                access: Arc::new(access.clone()),
                factory: factory.clone(),
                in_process: Arc::new(InProcessRefreshCoordinator::default()),
                distributed: Mutex::new(None),
                skew: DEFAULT_EXPIRY_SKEW,
                clock: clock(now_ms),
            },
            inner,
            transport,
            access,
            factory,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("utf-8 log output")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
