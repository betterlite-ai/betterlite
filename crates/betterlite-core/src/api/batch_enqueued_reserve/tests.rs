#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::error::Error as _;
use std::future::{Future, ready};
use std::slice::from_ref;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_openai::batches::records::{BatchObject, BatchStatus};
use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use secrecy::SecretString;

use super::keys::{COUNTER_KEY_PREFIX, RESERVATION_RECORD_PREFIX};
use super::metadata::resolve_enqueued_scopes;
use super::provider::canonical_provider_batch_id;
use super::response::BatchResponseView;
use super::store::{
    BatchEnqueuedReserveError, EnqueuedTokenCounters, LocalTokenCounters, ReservationScripts,
    ReserveScriptReply,
};
use super::{
    BATCH_ENQUEUED_LIMIT_KEY, BATCH_ENQUEUED_TTL_SECONDS, BatchEnqueuedPrincipal,
    BatchEnqueuedReserveInput, BatchEnqueuedReserveResult, BatchEnqueuedTokenOverLimit,
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope, BatchEnqueuedTokenSettings,
    BatchEnqueuedTokenStore, Monotonic, ReservationBackend, ScopeKey,
};

const KEY: &str = "sk-oracle-key";
const TEAM: &str = "team-oracle";
const CLOCK: f64 = 12.5;
const KEY_COUNTER: &str = "batch_enqueued_tokens:api_key:sk-oracle-key";
const TEAM_COUNTER: &str = "batch_enqueued_tokens:team:team-oracle";

#[derive(Debug, thiserror::Error)]
#[error("the scripted script call failed")]
struct ScriptFailure;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptCall {
    Reserve {
        key: String,
        tokens: i64,
        ttl_seconds: i64,
        limit: i64,
    },
    Refund {
        key: String,
        tokens: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReserveStep {
    Reply { status: i64, count: i64 },
    Fail,
}

#[derive(Clone)]
struct ScriptedScripts {
    calls: Arc<Mutex<Vec<ScriptCall>>>,
    replies: Arc<Mutex<VecDeque<ReserveStep>>>,
    refunds: Arc<Mutex<VecDeque<bool>>>,
}

impl ScriptedScripts {
    fn new(replies: Vec<ReserveStep>, refunds: Vec<bool>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            refunds: Arc::new(Mutex::new(VecDeque::from(refunds))),
        }
    }

    fn calls(&self) -> Vec<ScriptCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ReservationScripts for ScriptedScripts {
    type Error = ScriptFailure;

    fn reserve(
        &self,
        counter_key: &str,
        tokens: i64,
        ttl_seconds: i64,
        limit: i64,
    ) -> impl Future<Output = Result<ReserveScriptReply, ScriptFailure>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ScriptCall::Reserve {
                key: counter_key.to_owned(),
                tokens,
                ttl_seconds,
                limit,
            });
        let step = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        ready(match step {
            Some(ReserveStep::Reply { status, count }) => Ok(ReserveScriptReply { status, count }),
            Some(ReserveStep::Fail) | None => Err(ScriptFailure),
        })
    }

    fn refund(
        &self,
        counter_key: &str,
        tokens: i64,
    ) -> impl Future<Output = Result<(), ScriptFailure>> + Send {
        let refunded = self
            .refunds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ScriptCall::Refund {
                key: counter_key.to_owned(),
                tokens,
            });
        if refunded != Some(true) {
            return ready(Err(ScriptFailure));
        }
        ready(Ok(()))
    }
}

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<Level>>>);

impl Capture {
    fn levels(&self) -> Vec<Level> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(*event.metadata().level());
    }
}

fn capture() -> (Capture, DefaultGuard) {
    let captured = Capture::default();
    let guard = set_default(registry().with(captured.clone()));
    (captured, guard)
}

fn fixed_clock() -> Monotonic {
    Arc::new(|| CLOCK)
}

fn counters_at(time: &Arc<Mutex<f64>>) -> Arc<Mutex<LocalTokenCounters>> {
    let shared = Arc::clone(time);
    let cache = LocalTtlCache::with_clock(
        None,
        None,
        None,
        1024,
        Arc::new(move || *shared.lock().unwrap_or_else(PoisonError::into_inner)),
    )
    .expect("the local cache builds");
    Arc::new(Mutex::new(LocalTokenCounters { cache }))
}

fn store(
    scripts: Option<ScriptedScripts>,
    counters: Arc<Mutex<LocalTokenCounters>>,
) -> BatchEnqueuedTokenStore<ScriptedScripts, LocalTokenCounters> {
    BatchEnqueuedTokenStore {
        clock: fixed_clock(),
        scripts,
        counters,
        owner: "worker-a".to_owned(),
        settings: BatchEnqueuedTokenSettings::default(),
    }
}

fn number(value: i64) -> JsonData {
    JsonData::Number(serde_json::Number::from(value))
}

fn limit_only(limit: i64) -> BTreeMap<String, JsonData> {
    BTreeMap::from([(BATCH_ENQUEUED_LIMIT_KEY.to_owned(), number(limit))])
}

fn principal(key_limit: Option<i64>, team_limit: Option<i64>) -> BatchEnqueuedPrincipal {
    BatchEnqueuedPrincipal {
        api_key: Some(SecretString::from(KEY)),
        team_id: Some(TEAM.to_owned()),
        metadata: key_limit.map(limit_only),
        team_metadata: team_limit.map(limit_only),
    }
}

fn input(tokens: i64, principal: BatchEnqueuedPrincipal) -> BatchEnqueuedReserveInput {
    BatchEnqueuedReserveInput { tokens, principal }
}

fn scope(key: ScopeKey, value: &str, limit: i64) -> BatchEnqueuedTokenScope {
    BatchEnqueuedTokenScope {
        key,
        value: value.to_owned(),
        limit,
    }
}

fn reserve_call(key: &str, tokens: i64, limit: i64) -> ScriptCall {
    ScriptCall::Reserve {
        key: key.to_owned(),
        tokens,
        ttl_seconds: BATCH_ENQUEUED_TTL_SECONDS,
        limit,
    }
}

fn refund_call(key: &str, tokens: i64) -> ScriptCall {
    ScriptCall::Refund {
        key: key.to_owned(),
        tokens,
    }
}

fn stored(
    counters: &Arc<Mutex<LocalTokenCounters>>,
    value: &BatchEnqueuedTokenScope,
) -> Option<i64> {
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    EnqueuedTokenCounters::get(&mut *held, &counter_key(value)).expect("the counter reads")
}

fn seed(counters: &Arc<Mutex<LocalTokenCounters>>, value: &BatchEnqueuedTokenScope, count: i64) {
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    EnqueuedTokenCounters::set(
        &mut *held,
        &counter_key(value),
        count,
        BATCH_ENQUEUED_TTL_SECONDS,
    )
    .expect("the counter seeds");
}

fn reservation(outcome: BatchEnqueuedReserveResult) -> BatchEnqueuedTokenReservation {
    match outcome {
        BatchEnqueuedReserveResult::Reservation(reserved) => reserved,
        BatchEnqueuedReserveResult::OverLimit(found) => {
            panic!("expected a reservation, found {found:?}")
        }
    }
}

fn over_limit(outcome: BatchEnqueuedReserveResult) -> BatchEnqueuedTokenOverLimit {
    match outcome {
        BatchEnqueuedReserveResult::OverLimit(found) => found,
        BatchEnqueuedReserveResult::Reservation(found) => {
            panic!("expected an over-limit result, found {found:?}")
        }
    }
}

#[rstest]
#[case::both_scopes(Some(1000), Some(2000))]
#[case::key_only(Some(1000), None)]
#[case::team_only(None, Some(2000))]
#[tokio::test]
async fn key_team_reserve(#[case] key_limit: Option<i64>, #[case] team_limit: Option<i64>) {
    let mut replies = Vec::new();
    let mut expected = Vec::new();
    let mut expected_scopes = Vec::new();
    if let Some(limit) = key_limit {
        replies.push(ReserveStep::Reply {
            status: 1,
            count: 500,
        });
        expected.push(reserve_call(KEY_COUNTER, 500, limit));
        expected_scopes.push(scope(ScopeKey::ApiKey, KEY, limit));
    }
    if let Some(limit) = team_limit {
        replies.push(ReserveStep::Reply {
            status: 1,
            count: 500,
        });
        expected.push(reserve_call(TEAM_COUNTER, 500, limit));
        expected_scopes.push(scope(ScopeKey::Team, TEAM, limit));
    }
    let scripts = ScriptedScripts::new(replies, Vec::new());
    let time = Arc::new(Mutex::new(0.0_f64));
    let store = store(Some(scripts.clone()), counters_at(&time));
    let outcome = store
        .reserve(&input(500, principal(key_limit, team_limit)))
        .await
        .expect("the reservation succeeds");
    let reserved = reservation(outcome);
    assert_eq!(reserved.tokens, 500);
    assert_eq!(reserved.backend, ReservationBackend::Redis);
    assert_eq!(reserved.owner, "");
    assert_eq!(reserved.reserved_at_monotonic.to_bits(), CLOCK.to_bits());
    assert_eq!(reserved.scopes, expected_scopes);
    assert_eq!(scripts.calls(), expected);
}

#[rstest]
#[case::key_scope_over_limit(
    vec![ReserveStep::Reply { status: 0, count: 950 }],
    vec![],
    vec![reserve_call(KEY_COUNTER, 500, 1000)],
    ScopeKey::ApiKey,
    950
)]
#[case::team_scope_over_limit(
    vec![
        ReserveStep::Reply { status: 1, count: 400 },
        ReserveStep::Reply { status: 0, count: 1900 },
    ],
    vec![true],
    vec![
        reserve_call(KEY_COUNTER, 500, 1000),
        reserve_call(TEAM_COUNTER, 500, 2000),
        refund_call(KEY_COUNTER, 500),
    ],
    ScopeKey::Team,
    1900
)]
#[tokio::test]
async fn enqueued_limit_exceeded(
    #[case] replies: Vec<ReserveStep>,
    #[case] refunds: Vec<bool>,
    #[case] expected: Vec<ScriptCall>,
    #[case] expected_key: ScopeKey,
    #[case] expected_enqueued: i64,
) {
    let scripts = ScriptedScripts::new(replies, refunds);
    let time = Arc::new(Mutex::new(0.0_f64));
    let store = store(Some(scripts.clone()), counters_at(&time));
    let (captured, _guard) = capture();
    let outcome = store
        .reserve(&input(500, principal(Some(1000), Some(2000))))
        .await
        .expect("an over-limit answer is not an error");
    let over_limit = over_limit(outcome);
    assert_eq!(over_limit.scope.key, expected_key);
    assert_eq!(over_limit.enqueued, expected_enqueued);
    assert_eq!(scripts.calls(), expected);
    assert!(captured.levels().is_empty());
}

#[derive(Debug, Clone)]
enum MetadataValue {
    Absent,
    Present(JsonData),
}

#[rstest]
#[case::zero(MetadataValue::Present(number(0)), 1)]
#[case::negative(MetadataValue::Present(number(-3)), 1)]
#[case::numeric_text(MetadataValue::Present(JsonData::String("1000".to_owned())), 1)]
#[case::boolean(MetadataValue::Present(JsonData::Bool(true)), 1)]
#[case::fractional(
    MetadataValue::Present(JsonData::Number(
        serde_json::Number::from_f64(5.0).expect("a finite number"),
    )),
    1
)]
#[case::array(MetadataValue::Present(JsonData::Array(vec![number(10)])), 1)]
#[case::object(MetadataValue::Present(JsonData::Object(BTreeMap::new())), 1)]
#[case::null(MetadataValue::Present(JsonData::Null), 0)]
#[case::absent(MetadataValue::Absent, 0)]
fn invalid_limit_warning(#[case] metadata: MetadataValue, #[case] warnings: usize) {
    let (captured, _guard) = capture();
    let key_metadata = match metadata {
        MetadataValue::Absent => None,
        MetadataValue::Present(value) => Some(BTreeMap::from([(
            BATCH_ENQUEUED_LIMIT_KEY.to_owned(),
            value,
        )])),
    };
    let principal = BatchEnqueuedPrincipal {
        api_key: Some(SecretString::from(KEY)),
        team_id: Some(TEAM.to_owned()),
        metadata: key_metadata,
        team_metadata: Some(limit_only(2000)),
    };
    let scopes = resolve_enqueued_scopes(&principal);
    assert_eq!(scopes, vec![scope(ScopeKey::Team, TEAM, 2000)]);
    assert_eq!(captured.levels(), vec![Level::WARN; warnings]);
}

#[test]
fn empty_metadata_is_silent() {
    let (captured, _guard) = capture();
    let principal = BatchEnqueuedPrincipal {
        api_key: Some(SecretString::from(KEY)),
        team_id: Some(TEAM.to_owned()),
        metadata: Some(BTreeMap::new()),
        team_metadata: Some(BTreeMap::new()),
    };
    assert!(resolve_enqueued_scopes(&principal).is_empty());
    assert!(captured.levels().is_empty());
}

#[test]
fn empty_scope_values_are_skipped() {
    let principal = BatchEnqueuedPrincipal {
        api_key: Some(SecretString::from("")),
        team_id: Some(String::new()),
        metadata: Some(limit_only(1000)),
        team_metadata: Some(limit_only(2000)),
    };
    assert!(resolve_enqueued_scopes(&principal).is_empty());
}

#[rstest]
#[case::refund_succeeds(true, 0)]
#[case::refund_fails(false, 1)]
#[tokio::test]
async fn partial_reserve_rollback(#[case] refund_ok: bool, #[case] warnings: usize) {
    let scripts = ScriptedScripts::new(
        vec![
            ReserveStep::Reply {
                status: 1,
                count: 100,
            },
            ReserveStep::Reply {
                status: 0,
                count: 1900,
            },
        ],
        vec![refund_ok],
    );
    let time = Arc::new(Mutex::new(0.0_f64));
    let store = store(Some(scripts.clone()), counters_at(&time));
    let (captured, _guard) = capture();
    let outcome = store
        .reserve(&input(500, principal(Some(1000), Some(2000))))
        .await
        .expect("an over-limit answer is not an error");
    let over_limit = over_limit(outcome);
    assert_eq!(over_limit.scope, scope(ScopeKey::Team, TEAM, 2000));
    assert_eq!(over_limit.enqueued, 1900);
    assert_eq!(
        scripts.calls(),
        vec![
            reserve_call(KEY_COUNTER, 500, 1000),
            reserve_call(TEAM_COUNTER, 500, 2000),
            refund_call(KEY_COUNTER, 500),
        ]
    );
    assert_eq!(captured.levels(), vec![Level::WARN; warnings]);
}

#[tokio::test]
async fn partial_rollback_refunds_every_earlier_scope() {
    let scripts = ScriptedScripts::new(
        vec![
            ReserveStep::Reply {
                status: 1,
                count: 10,
            },
            ReserveStep::Reply {
                status: 1,
                count: 20,
            },
            ReserveStep::Reply {
                status: 0,
                count: 30,
            },
        ],
        vec![true, true],
    );
    let time = Arc::new(Mutex::new(0.0_f64));
    let store = store(Some(scripts.clone()), counters_at(&time));
    let scopes = vec![
        scope(ScopeKey::ApiKey, KEY, 1000),
        scope(ScopeKey::Team, TEAM, 2000),
        scope(ScopeKey::Team, "team-late", 3000),
    ];
    let outcome = store
        .reserve_scopes(500, &scopes)
        .await
        .expect("an over-limit answer is not an error");
    assert_eq!(over_limit(outcome).scope.value, "team-late");
    assert_eq!(
        scripts.calls(),
        vec![
            reserve_call(KEY_COUNTER, 500, 1000),
            reserve_call(TEAM_COUNTER, 500, 2000),
            reserve_call("batch_enqueued_tokens:team:team-late", 500, 3000),
            refund_call(KEY_COUNTER, 500),
            refund_call(TEAM_COUNTER, 500),
        ]
    );
}

#[rstest]
#[case::script_failure(vec![ReserveStep::Fail], 0, 500, 1)]
#[case::no_script_configured(vec![], 300, 800, 0)]
#[case::exactly_at_limit(vec![], 500, 1000, 0)]
#[tokio::test]
async fn redis_fallback_memory(
    #[case] replies: Vec<ReserveStep>,
    #[case] seed_key: i64,
    #[case] expected_key: i64,
    #[case] warnings: usize,
) {
    let scripts = if replies.is_empty() {
        None
    } else {
        Some(ScriptedScripts::new(replies, Vec::new()))
    };
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let key_scope = scope(ScopeKey::ApiKey, KEY, 1000);
    let team_scope = scope(ScopeKey::Team, TEAM, 2000);
    seed(&counters, &key_scope, seed_key);
    seed(&counters, &team_scope, 100);
    let store = store(scripts.clone(), Arc::clone(&counters));
    let (captured, _guard) = capture();
    let outcome = store
        .reserve(&input(500, principal(Some(1000), Some(2000))))
        .await
        .expect("the memory fallback answers");
    let reserved = reservation(outcome);
    assert_eq!(reserved.tokens, 500);
    assert_eq!(reserved.backend, ReservationBackend::Memory);
    assert_eq!(reserved.owner, "worker-a");
    assert_eq!(reserved.reserved_at_monotonic.to_bits(), CLOCK.to_bits());
    assert_eq!(reserved.scopes, vec![key_scope.clone(), team_scope.clone()]);
    assert_eq!(stored(&counters, &key_scope), Some(expected_key));
    assert_eq!(stored(&counters, &team_scope), Some(600));
    assert_eq!(captured.levels(), vec![Level::WARN; warnings]);
    if let Some(fake) = scripts {
        assert_eq!(fake.calls(), vec![reserve_call(KEY_COUNTER, 500, 1000)]);
    }
}

#[tokio::test]
async fn memory_over_limit_writes_nothing() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let key_scope = scope(ScopeKey::ApiKey, KEY, 1000);
    let team_scope = scope(ScopeKey::Team, TEAM, 2000);
    seed(&counters, &key_scope, 800);
    seed(&counters, &team_scope, 100);
    let store = store(None, Arc::clone(&counters));
    let outcome = store
        .reserve(&input(300, principal(Some(1000), Some(2000))))
        .await
        .expect("an over-limit answer is not an error");
    let over_limit = over_limit(outcome);
    assert_eq!(over_limit.scope, key_scope);
    assert_eq!(over_limit.enqueued, 800);
    assert_eq!(stored(&counters, &key_scope), Some(800));
    assert_eq!(stored(&counters, &team_scope), Some(100));
}

#[tokio::test]
async fn memory_token_overflow_counts_as_over_limit() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let key_scope = scope(ScopeKey::ApiKey, KEY, i64::MAX);
    seed(&counters, &key_scope, 1);
    let store = store(None, Arc::clone(&counters));
    let outcome = store
        .reserve_scopes(i64::MAX, from_ref(&key_scope))
        .await
        .expect("an over-limit answer is not an error");
    let over_limit = over_limit(outcome);
    assert_eq!(over_limit.enqueued, 1);
    assert_eq!(stored(&counters, &key_scope), Some(1));
}

#[tokio::test]
async fn local_counters_expire_with_the_ttl() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let store = store(None, Arc::clone(&counters));
    let first = store
        .reserve(&input(500, principal(Some(1000), Some(1000))))
        .await
        .expect("the first reservation is under the limit");
    assert!(matches!(first, BatchEnqueuedReserveResult::Reservation(_)));
    *time.lock().unwrap_or_else(PoisonError::into_inner) = 691_201.0_f64;
    let second = store
        .reserve(&input(600, principal(Some(1000), Some(1000))))
        .await
        .expect("the expired counter grants the reservation");
    assert!(matches!(second, BatchEnqueuedReserveResult::Reservation(_)));
}

#[rstest]
#[case::zero_tokens(0, false)]
#[case::negative_tokens(-7, false)]
#[case::empty_scopes(500, true)]
#[tokio::test]
async fn no_op_reservation(#[case] tokens: i64, #[case] empty_scopes: bool) {
    let scripts = ScriptedScripts::new(
        vec![
            ReserveStep::Reply {
                status: 1,
                count: 1,
            },
            ReserveStep::Reply {
                status: 1,
                count: 1,
            },
        ],
        Vec::new(),
    );
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let store = store(Some(scripts.clone()), Arc::clone(&counters));
    let (captured, _guard) = capture();
    let scopes = if empty_scopes {
        Vec::new()
    } else {
        vec![scope(ScopeKey::ApiKey, KEY, 1000)]
    };
    let outcome = store
        .reserve_scopes(tokens, &scopes)
        .await
        .expect("a no-op reservation succeeds");
    let reserved = reservation(outcome);
    assert_eq!(reserved.tokens, tokens.max(0));
    assert_eq!(reserved.scopes, scopes);
    assert_eq!(reserved.backend, ReservationBackend::Redis);
    assert_eq!(reserved.owner, "");
    assert_eq!(reserved.reserved_at_monotonic.to_bits(), CLOCK.to_bits());
    assert!(scripts.calls().is_empty());
    assert!(captured.levels().is_empty());
    if !empty_scopes {
        assert!(stored(&counters, &scope(ScopeKey::ApiKey, KEY, 1000)).is_none());
    }
}

#[test]
fn reservation_keys_match_the_source_format() {
    assert_eq!(
        counter_key(&scope(ScopeKey::ApiKey, "sk-a", 10)),
        "batch_enqueued_tokens:api_key:sk-a"
    );
    assert_eq!(
        counter_key(&scope(ScopeKey::Team, "team-a", 10)),
        "batch_enqueued_tokens:team:team-a"
    );
    assert_eq!(
        format!("{RESERVATION_RECORD_PREFIX}batch-1"),
        "batch_enqueued_token_reservation:batch-1"
    );
}

fn counter_key(value: &BatchEnqueuedTokenScope) -> String {
    format!("{COUNTER_KEY_PREFIX}{}:{}", value.key.as_str(), value.value)
}

#[rstest]
#[case::plain_provider_id("batch-abc123".to_owned(), "batch-abc123")]
#[case::unpaired_batch_id("b-42".to_owned(), "b-42")]
#[case::header_and_model(
    format!("batch_{}", URL_SAFE_NO_PAD.encode("litellm:b-1;model,m")),
    "b-1"
)]
#[case::batch_marker(
    URL_SAFE_NO_PAD.encode("litellm_proxy;model_id:m;llm_batch_id:b-1;llm_output_file_id:f"),
    "b-1"
)]
#[case::generic_response_marker(
    URL_SAFE_NO_PAD.encode("litellm_proxy;model_id:m;generic_response_id:g-9"),
    "g-9"
)]
#[case::managed_id_without_markers(
    URL_SAFE_NO_PAD.encode("litellm_proxy;model_id:m;unified_id,u"),
    "litellm_proxy;model_id:m;unified_id,u"
)]
fn canonical_provider_batch_id_cases(#[case] batch_id: String, #[case] expected: &str) {
    assert_eq!(canonical_provider_batch_id(&batch_id), expected);
}

#[test]
fn canonical_provider_batch_id_keeps_an_unparsable_marker_id() {
    let encoded = URL_SAFE_NO_PAD.encode("litellm_proxy;model_id:m;xllm_batch_id");
    assert_eq!(canonical_provider_batch_id(&encoded), encoded);
}

#[test]
fn batch_response_view_projects_id_and_status() {
    let response: BatchObject = serde_json::from_str(
        r#"{
            "id": "batch_abc",
            "completion_window": "24h",
            "created_at": 1,
            "endpoint": "/v1/chat/completions",
            "input_file_id": "file-1",
            "object": "batch",
            "status": "completed"
        }"#,
    )
    .expect("the batch parses");
    assert_eq!(
        BatchResponseView::from(&response),
        BatchResponseView {
            id: "batch_abc".to_owned(),
            status: BatchStatus::Completed,
        }
    );
}

#[test]
fn settings_default_is_the_source_ttl() {
    assert_eq!(BATCH_ENQUEUED_TTL_SECONDS, 691_200);
    assert_eq!(
        BatchEnqueuedTokenSettings::default().ttl_seconds,
        BATCH_ENQUEUED_TTL_SECONDS
    );
}

#[test]
fn stored_non_integer_counter_is_an_error() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    held.cache
        .set(KEY_COUNTER, CacheValue::Boolean(true), Some(691_200.0_f64))
        .expect("the wrong value stores");
    let error = EnqueuedTokenCounters::get(&mut *held, KEY_COUNTER)
        .expect_err("a boolean counter is not an integer");
    drop(held);
    assert!(matches!(error, BatchEnqueuedReserveError::StoredCounter));
}

#[test]
fn a_negative_ttl_is_out_of_range() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    let error = EnqueuedTokenCounters::set(&mut *held, KEY_COUNTER, 5, -1)
        .expect_err("a negative ttl cannot reach a duration");
    drop(held);
    assert!(matches!(
        error,
        BatchEnqueuedReserveError::TtlOutOfRange { .. }
    ));
    assert!(error.source().is_some());
}
