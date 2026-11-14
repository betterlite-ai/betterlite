#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::record::{ReservationRecord, record_ttl};
use super::store::ReservationRecordScripts;
use super::{
    BATCH_ENQUEUED_REFUND_STATUSES, BatchEnqueuedRefundError, BatchEnqueuedRefundInput,
    BatchEnqueuedRefundResult, EnqueuedTokenRefund, PostCallOutcome,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_enqueued_reserve::store::{
    EnqueuedTokenCounters, LocalTokenCounters, ReservationScripts, ReserveScriptReply,
};
use crate::api::batch_enqueued_reserve::{
    BatchEnqueuedTokenReservation, BatchEnqueuedTokenScope, BatchEnqueuedTokenSettings, Monotonic,
    ReservationBackend, ScopeKey,
};

const KEY: &str = "sk-oracle-key";
const TEAM: &str = "team-oracle";
const KEY_COUNTER: &str = "batch_enqueued_tokens:api_key:sk-oracle-key";
const TEAM_COUNTER: &str = "batch_enqueued_tokens:team:team-oracle";
const RECORD_KEY: &str = "batch_enqueued_token_reservation:batch-1";
const NOW: f64 = 20.0;
const RESERVED_AT: f64 = 12.5;
const TTL: i64 = 691_200;
const TTL_FLOAT: f64 = 691_200.0;
const SAVED_TTL: i64 = 691_192;

#[derive(Debug, thiserror::Error)]
#[error("the scripted script call failed: {message}")]
struct ScriptFailure {
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptCall {
    Refund {
        key: String,
        tokens: i64,
    },
    Save {
        key: String,
        record: String,
        ttl_seconds: i64,
    },
    Pop {
        key: String,
        ttl_seconds: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Done,
    Fail(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PopReply {
    Record(String),
    Tombstone,
    Absent,
    Fail(String),
}

#[derive(Clone)]
struct ScriptedScripts {
    calls: Arc<Mutex<Vec<ScriptCall>>>,
    refunds: Arc<Mutex<VecDeque<Step>>>,
    saves: Arc<Mutex<VecDeque<Step>>>,
    pops: Arc<Mutex<VecDeque<PopReply>>>,
    yielding_pops: bool,
}

impl ScriptedScripts {
    fn new(refunds: Vec<Step>, saves: Vec<Step>, pops: Vec<PopReply>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            refunds: Arc::new(Mutex::new(VecDeque::from(refunds))),
            saves: Arc::new(Mutex::new(VecDeque::from(saves))),
            pops: Arc::new(Mutex::new(VecDeque::from(pops))),
            yielding_pops: false,
        }
    }

    fn calls(&self) -> Vec<ScriptCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn refund_calls(&self) -> Vec<ScriptCall> {
        self.calls()
            .into_iter()
            .filter(|call| matches!(call, ScriptCall::Refund { .. }))
            .collect()
    }

    fn pop_calls(&self) -> Vec<ScriptCall> {
        self.calls()
            .into_iter()
            .filter(|call| matches!(call, ScriptCall::Pop { .. }))
            .collect()
    }

    fn record_call(&self, call: ScriptCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    fn take_reply_from(&self, queue: &Mutex<VecDeque<Step>>) -> Result<(), ScriptFailure> {
        let step = queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match step {
            Some(Step::Done) => Ok(()),
            Some(Step::Fail(message)) => Err(ScriptFailure { message }),
            None => Err(ScriptFailure {
                message: "the scripted queue is empty".to_owned(),
            }),
        }
    }
}

impl ReservationScripts for ScriptedScripts {
    type Error = ScriptFailure;

    fn reserve(
        &self,
        _counter_key: &str,
        _tokens: i64,
        _ttl_seconds: i64,
        _limit: i64,
    ) -> impl Future<Output = Result<ReserveScriptReply, ScriptFailure>> + Send {
        let failure = ScriptFailure {
            message: "the refund slice never reserves".to_owned(),
        };
        ready(Err(failure))
    }

    fn refund(
        &self,
        counter_key: &str,
        tokens: i64,
    ) -> impl Future<Output = Result<(), ScriptFailure>> + Send {
        self.record_call(ScriptCall::Refund {
            key: counter_key.to_owned(),
            tokens,
        });
        ready(self.take_reply_from(&self.refunds))
    }
}

impl ReservationRecordScripts for ScriptedScripts {
    fn save(
        &self,
        record_key: &str,
        record: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<(), ScriptFailure>> + Send {
        self.record_call(script_save(record_key, record, ttl_seconds));
        let reply = self.take_reply_from(&self.saves);
        ready(reply)
    }

    fn pop(
        &self,
        record_key: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, ScriptFailure>> + Send {
        self.record_call(ScriptCall::Pop {
            key: record_key.to_owned(),
            ttl_seconds,
        });
        let reply = self
            .pops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        let outcome = match reply {
            Some(PopReply::Record(record)) => Ok(Some(record.into_bytes())),
            Some(PopReply::Tombstone) => Ok(Some(Vec::new())),
            Some(PopReply::Absent) => Ok(None),
            Some(PopReply::Fail(message)) => Err(ScriptFailure { message }),
            None => Err(ScriptFailure {
                message: "the scripted pop queue is empty".to_owned(),
            }),
        };
        let yields = self.yielding_pops;
        async move {
            if yields {
                tokio::task::yield_now().await;
            }
            outcome
        }
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
    Arc::new(|| NOW)
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

fn refund_service(
    scripts: Option<ScriptedScripts>,
    counters: Arc<Mutex<LocalTokenCounters>>,
) -> EnqueuedTokenRefund<ScriptedScripts, LocalTokenCounters> {
    EnqueuedTokenRefund {
        clock: fixed_clock(),
        scripts,
        store: counters,
        owner: "worker-a".to_owned(),
        settings: BatchEnqueuedTokenSettings::default(),
        limits: ApiLimits::default(),
    }
}

fn tight_limits() -> ApiLimits {
    let defaults = ApiLimits::default();
    ApiLimits {
        max_json_bytes: 4,
        max_event_bytes: defaults.max_event_bytes,
        max_items: defaults.max_items,
    }
}

fn scope(key: ScopeKey, value: &str, limit: i64) -> BatchEnqueuedTokenScope {
    BatchEnqueuedTokenScope {
        key,
        value: value.to_owned(),
        limit,
    }
}

fn key_scope() -> BatchEnqueuedTokenScope {
    scope(ScopeKey::ApiKey, KEY, 1000)
}

fn team_scope() -> BatchEnqueuedTokenScope {
    scope(ScopeKey::Team, TEAM, 2000)
}

fn record(
    tokens: i64,
    backend: ReservationBackend,
    owner: &str,
    scopes: Vec<BatchEnqueuedTokenScope>,
) -> ReservationRecord {
    ReservationRecord {
        tokens,
        scopes,
        backend,
        owner: owner.to_owned(),
        reserved_at_monotonic: RESERVED_AT,
    }
}

fn redis_record(tokens: i64, scopes: Vec<BatchEnqueuedTokenScope>) -> ReservationRecord {
    record(tokens, ReservationBackend::Redis, "", scopes)
}

fn record_text(reservation: &ReservationRecord) -> String {
    serde_json::to_string(reservation).expect("the record serializes")
}

fn counter_key(value: &BatchEnqueuedTokenScope) -> String {
    format!(
        "batch_enqueued_tokens:{}:{}",
        value.key.as_str(),
        value.value
    )
}

fn seed_counter(
    counters: &Arc<Mutex<LocalTokenCounters>>,
    value: &BatchEnqueuedTokenScope,
    count: i64,
) {
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    EnqueuedTokenCounters::set(&mut *held, &counter_key(value), count, TTL)
        .expect("the counter seeds");
}

fn stored(
    counters: &Arc<Mutex<LocalTokenCounters>>,
    value: &BatchEnqueuedTokenScope,
) -> Option<i64> {
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    EnqueuedTokenCounters::get(&mut *held, &counter_key(value)).expect("the counter reads")
}

fn seed_record(counters: &Arc<Mutex<LocalTokenCounters>>, text: &str) {
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    held.cache
        .set(
            RECORD_KEY,
            CacheValue::Text(text.to_owned()),
            Some(TTL_FLOAT),
        )
        .expect("the record seeds");
}

fn stored_tokens(counters: &Arc<Mutex<LocalTokenCounters>>) -> Option<i64> {
    let mut held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    match held.cache.get(RECORD_KEY).expect("the record reads") {
        Some(CacheValue::Object(fields)) => match fields.get("tokens") {
            Some(CacheValue::Integer(tokens)) => Some(*tokens),
            _ => None,
        },
        _ => None,
    }
}

fn record_expiry(counters: &Arc<Mutex<LocalTokenCounters>>) -> Option<f64> {
    let held = counters.lock().unwrap_or_else(PoisonError::into_inner);
    held.cache
        .ttl_entries()
        .into_iter()
        .find(|(key, _expiry)| key == RECORD_KEY)
        .map(|(_key, expiry)| expiry)
}

fn script_save(record_key: &str, record: &str, ttl_seconds: i64) -> ScriptCall {
    ScriptCall::Save {
        key: record_key.to_owned(),
        record: record.to_owned(),
        ttl_seconds,
    }
}

fn saved_call(reservation: &ReservationRecord, ttl_seconds: i64) -> ScriptCall {
    ScriptCall::Save {
        key: RECORD_KEY.to_owned(),
        record: record_text(reservation),
        ttl_seconds,
    }
}

fn pop_call() -> ScriptCall {
    ScriptCall::Pop {
        key: RECORD_KEY.to_owned(),
        ttl_seconds: TTL,
    }
}

fn refund_call(key: &str, tokens: i64) -> ScriptCall {
    ScriptCall::Refund {
        key: key.to_owned(),
        tokens,
    }
}

fn post_call(
    batch_id: &str,
    status: &str,
    pending: Option<ReservationRecord>,
) -> BatchEnqueuedRefundInput {
    BatchEnqueuedRefundInput::PostCall {
        batch_id: batch_id.to_owned(),
        status: status.to_owned(),
        pending,
    }
}

fn managed_batch_id() -> String {
    let encoded = URL_SAFE_NO_PAD
        .encode("litellm_proxy;model_id:m;llm_batch_id:batch-1;llm_output_file_id:f");
    format!("batch_{encoded}")
}

#[tokio::test]
async fn terminal_refund_once() {
    let pending = redis_record(500, vec![key_scope(), team_scope()]);
    let scripts = ScriptedScripts::new(
        vec![Step::Done, Step::Done],
        vec![Step::Done],
        vec![PopReply::Record(record_text(&pending))],
    );
    let time = Arc::new(Mutex::new(0.0_f64));
    let service = refund_service(Some(scripts.clone()), counters_at(&time));
    let (captured, _guard) = capture();
    let outcome = service
        .run(&post_call(
            &managed_batch_id(),
            "completed",
            Some(pending.clone()),
        ))
        .await
        .expect("the terminal post-call flow answers");
    assert_eq!(
        outcome,
        BatchEnqueuedRefundResult::PostCall(PostCallOutcome {
            batch_id: "batch-1".to_owned(),
            saved: true,
            refunded: true,
        })
    );
    assert_eq!(
        scripts.calls(),
        vec![
            saved_call(&pending, SAVED_TTL),
            pop_call(),
            refund_call(KEY_COUNTER, 500),
            refund_call(TEAM_COUNTER, 500),
        ]
    );
    assert!(captured.levels().is_empty());
}

#[rstest]
#[case::completed("completed", true)]
#[case::complete("complete", true)]
#[case::failed("failed", true)]
#[case::expired("expired", true)]
#[case::cancelled("cancelled", true)]
#[case::cancelling_uppercase("Cancelling", true)]
#[case::in_progress("in_progress", false)]
#[tokio::test]
async fn cancelling_refund(#[case] status: &str, #[case] refunded: bool) {
    let popped = redis_record(500, vec![key_scope()]);
    let scripts = ScriptedScripts::new(
        vec![Step::Done],
        Vec::new(),
        vec![PopReply::Record(record_text(&popped))],
    );
    let time = Arc::new(Mutex::new(0.0_f64));
    let service = refund_service(Some(scripts.clone()), counters_at(&time));
    let (captured, _guard) = capture();
    let outcome = service
        .run(&post_call("batch-1", status, None))
        .await
        .expect("the post-call flow answers");
    assert_eq!(
        outcome,
        BatchEnqueuedRefundResult::PostCall(PostCallOutcome {
            batch_id: "batch-1".to_owned(),
            saved: false,
            refunded,
        })
    );
    let expected = if refunded {
        vec![pop_call(), refund_call(KEY_COUNTER, 500)]
    } else {
        Vec::new()
    };
    assert_eq!(scripts.calls(), expected);
    assert!(captured.levels().is_empty());
}

#[tokio::test]
async fn reservation_tombstone() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let left_behind = redis_record(500, vec![key_scope()]);
    seed_record(&counters, &record_text(&left_behind));
    let scripts = ScriptedScripts::new(
        Vec::new(),
        Vec::new(),
        vec![PopReply::Tombstone, PopReply::Absent],
    );
    let service = refund_service(Some(scripts.clone()), Arc::clone(&counters));
    let (captured, _guard) = capture();
    let input = BatchEnqueuedRefundInput::Pop {
        batch_id: "batch-1".to_owned(),
    };
    let first = service
        .run(&input)
        .await
        .expect("the tombstoned pop answers");
    assert_eq!(first, BatchEnqueuedRefundResult::Popped(None));
    assert!(stored_tokens(&counters).is_none());
    let second = service.run(&input).await.expect("the second poll answers");
    assert_eq!(second, BatchEnqueuedRefundResult::Popped(None));
    assert_eq!(scripts.calls(), vec![pop_call(), pop_call()]);
    assert!(captured.levels().is_empty());
}

#[rstest]
#[case::decrements(Some(800), Some(300))]
#[case::exactly_drained(Some(500), None)]
#[case::over_drawn(Some(400), None)]
#[case::absent_counter(None, None)]
#[tokio::test]
async fn local_backend_refund(#[case] current: Option<i64>, #[case] remaining: Option<i64>) {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let counter = key_scope();
    if let Some(current) = current {
        seed_counter(&counters, &counter, current);
    }
    let scripts = ScriptedScripts::new(Vec::new(), Vec::new(), Vec::new());
    let service = refund_service(Some(scripts.clone()), Arc::clone(&counters));
    let (captured, _guard) = capture();
    let reservation = record(
        500,
        ReservationBackend::Memory,
        "worker-a",
        vec![counter.clone()],
    );
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Refund { reservation })
        .await
        .expect("the memory refund answers");
    assert_eq!(outcome, BatchEnqueuedRefundResult::Refunded);
    assert_eq!(stored(&counters, &counter), remaining);
    if remaining.is_some() {
        let entries = counters
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cache
            .ttl_entries();
        assert!(entries.contains(&(KEY_COUNTER.to_owned(), TTL_FLOAT)));
    }
    assert!(scripts.calls().is_empty());
    assert!(captured.levels().is_empty());
}

#[tokio::test]
async fn local_refund_with_a_negative_ttl_fails_in_the_store() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let counter = key_scope();
    seed_counter(&counters, &counter, 800);
    let mut service = refund_service(None, Arc::clone(&counters));
    service.settings.ttl_seconds = -1;
    let reservation = record(500, ReservationBackend::Memory, "worker-a", vec![counter]);
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Refund { reservation })
        .await;
    let error = outcome.expect_err("the negative ttl is rejected");
    assert!(matches!(error, BatchEnqueuedRefundError::Store(_)));
    assert!(std::error::Error::source(&error).is_some());
}

#[tokio::test]
async fn local_pop_with_a_broken_clock_fails_in_the_cache() {
    let time = Arc::new(Mutex::new(f64::NAN));
    let service = refund_service(None, counters_at(&time));
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Pop {
            batch_id: "batch-1".to_owned(),
        })
        .await;
    let error = outcome.expect_err("the broken clock is rejected");
    assert!(matches!(error, BatchEnqueuedRefundError::Cache(_)));
    assert!(std::error::Error::source(&error).is_some());
}

#[tokio::test]
async fn duplicate_refund_race() {
    let popped = redis_record(500, vec![key_scope()]);
    let mut scripts = ScriptedScripts::new(
        vec![Step::Done],
        Vec::new(),
        vec![PopReply::Record(record_text(&popped)), PopReply::Tombstone],
    );
    scripts.yielding_pops = true;
    let time = Arc::new(Mutex::new(0.0_f64));
    let service = refund_service(Some(scripts.clone()), counters_at(&time));
    let (captured, _guard) = capture();
    let input = post_call("batch-1", "completed", None);
    let (first, second) = tokio::join!(service.run(&input), service.run(&input));
    let outcomes = [
        first.expect("the first poll answers"),
        second.expect("the second poll answers"),
    ];
    let refunded = outcomes
        .into_iter()
        .filter(|outcome| {
            matches!(
                outcome,
                BatchEnqueuedRefundResult::PostCall(PostCallOutcome { refunded: true, .. })
            )
        })
        .count();
    assert_eq!(refunded, 1);
    assert_eq!(scripts.refund_calls(), vec![refund_call(KEY_COUNTER, 500)]);
    assert_eq!(scripts.pop_calls(), vec![pop_call(), pop_call()]);
    assert!(captured.levels().is_empty());
}

#[tokio::test]
async fn foreign_worker_memory_refund_is_skipped() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let counter = key_scope();
    seed_counter(&counters, &counter, 800);
    let scripts = ScriptedScripts::new(vec![Step::Done], Vec::new(), Vec::new());
    let service = refund_service(Some(scripts.clone()), Arc::clone(&counters));
    let (captured, _guard) = capture();
    let reservation = record(
        500,
        ReservationBackend::Memory,
        "worker-b",
        vec![counter.clone()],
    );
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Refund { reservation })
        .await
        .expect("the foreign refund answers");
    assert_eq!(outcome, BatchEnqueuedRefundResult::Refunded);
    assert_eq!(stored(&counters, &counter), Some(800));
    assert_eq!(scripts.calls(), Vec::new());
    assert_eq!(captured.levels(), vec![Level::WARN]);
}

#[tokio::test]
async fn redis_refund_without_a_client_warns() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let counter = key_scope();
    seed_counter(&counters, &counter, 800);
    let service = refund_service(None, Arc::clone(&counters));
    let (captured, _guard) = capture();
    let reservation = redis_record(500, vec![counter.clone()]);
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Refund { reservation })
        .await
        .expect("the clientless refund answers");
    assert_eq!(outcome, BatchEnqueuedRefundResult::Refunded);
    assert_eq!(stored(&counters, &counter), Some(800));
    assert_eq!(captured.levels(), vec![Level::WARN]);
}

#[tokio::test]
async fn redis_refund_failure_stops_at_the_failed_scope() {
    let scripts = ScriptedScripts::new(vec![Step::Fail("down".to_owned())], Vec::new(), Vec::new());
    let time = Arc::new(Mutex::new(0.0_f64));
    let service = refund_service(Some(scripts.clone()), counters_at(&time));
    let (captured, _guard) = capture();
    let reservation = redis_record(500, vec![key_scope(), team_scope()]);
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Refund { reservation })
        .await
        .expect("the failed refund answers");
    assert_eq!(outcome, BatchEnqueuedRefundResult::Refunded);
    assert_eq!(scripts.calls(), vec![refund_call(KEY_COUNTER, 500)]);
    assert_eq!(captured.levels(), vec![Level::WARN]);
}

#[rstest]
#[case::zero_tokens(0, false)]
#[case::negative_tokens(-7, false)]
#[case::empty_scopes(500, true)]
#[tokio::test]
async fn no_op_refund_skips_everything(#[case] tokens: i64, #[case] empty_scopes: bool) {
    let scripts = ScriptedScripts::new(vec![Step::Done], vec![Step::Done], vec![PopReply::Absent]);
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let counter = key_scope();
    seed_counter(&counters, &counter, 800);
    let service = refund_service(Some(scripts.clone()), Arc::clone(&counters));
    let (captured, _guard) = capture();
    let scopes = if empty_scopes {
        Vec::new()
    } else {
        vec![counter.clone()]
    };
    let reservation = record(tokens, ReservationBackend::Memory, "worker-a", scopes);
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Refund { reservation })
        .await
        .expect("a no-op refund answers");
    assert_eq!(outcome, BatchEnqueuedRefundResult::Refunded);
    assert_eq!(stored(&counters, &counter), Some(800));
    assert!(scripts.calls().is_empty());
    assert!(captured.levels().is_empty());
}

#[tokio::test]
async fn save_falls_back_to_local_when_the_script_fails() {
    let scripts = ScriptedScripts::new(Vec::new(), vec![Step::Fail("down".to_owned())], Vec::new());
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let service = refund_service(Some(scripts.clone()), Arc::clone(&counters));
    let (captured, _guard) = capture();
    let reservation = redis_record(500, vec![key_scope()]);
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Save {
            batch_id: "batch-1".to_owned(),
            reservation: reservation.clone(),
        })
        .await
        .expect("the save answers");
    assert_eq!(outcome, BatchEnqueuedRefundResult::Saved);
    assert_eq!(scripts.calls(), vec![saved_call(&reservation, SAVED_TTL)]);
    assert_eq!(stored_tokens(&counters), Some(500));
    assert_eq!(record_expiry(&counters), Some(691_192.0_f64));
    assert_eq!(captured.levels(), vec![Level::WARN]);
}

#[rstest]
#[case::just_reserved(20.0, TTL)]
#[case::behind_the_clock(20.5, TTL)]
#[case::one_second(19.0, 691_199)]
#[case::default_stamp(12.5, SAVED_TTL)]
#[case::nearly_expired(-690_800.0, 380)]
#[tokio::test]
async fn save_shortens_the_record_ttl(#[case] reserved_at: f64, #[case] ttl_seconds: i64) {
    let scripts = ScriptedScripts::new(Vec::new(), vec![Step::Done], Vec::new());
    let time = Arc::new(Mutex::new(0.0_f64));
    let service = refund_service(Some(scripts.clone()), counters_at(&time));
    let mut reservation = redis_record(500, vec![key_scope()]);
    reservation.reserved_at_monotonic = reserved_at;
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Save {
            batch_id: "batch-1".to_owned(),
            reservation: reservation.clone(),
        })
        .await
        .expect("the save answers");
    assert_eq!(outcome, BatchEnqueuedRefundResult::Saved);
    assert_eq!(scripts.calls(), vec![saved_call(&reservation, ttl_seconds)]);
}

#[rstest]
#[case::at_the_ttl(691_200.0, 1)]
#[case::past_the_ttl(700_000.0, 1)]
#[case::half_a_second(0.5, 691_199)]
#[case::negative_elapsed(-1.5, 691_201)]
#[case::not_a_number(f64::NAN, 1)]
#[case::infinite(f64::INFINITY, 1)]
#[case::zero_elapsed(0.0, TTL)]
fn record_ttl_clamps(#[case] elapsed_seconds: f64, #[case] expected: i64) {
    assert_eq!(record_ttl(TTL, elapsed_seconds), expected);
}

#[rstest]
#[case::missing_scopes(r#"{"tokens":500,"reserved_at_monotonic":1}"#)]
#[case::not_json("not a record")]
#[case::wrong_shape(r#"["tokens"]"#)]
#[tokio::test]
async fn malformed_record_is_discarded(#[case] text: &str) {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    seed_record(&counters, text);
    let service = refund_service(None, Arc::clone(&counters));
    let (captured, _guard) = capture();
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Pop {
            batch_id: "batch-1".to_owned(),
        })
        .await
        .expect("a malformed record is not an error");
    assert_eq!(outcome, BatchEnqueuedRefundResult::Popped(None));
    assert_eq!(captured.levels(), vec![Level::WARN]);
}

#[tokio::test]
async fn oversized_record_is_discarded() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    seed_record(
        &counters,
        &record_text(&redis_record(500, vec![key_scope()])),
    );
    let mut service = refund_service(None, Arc::clone(&counters));
    service.limits = tight_limits();
    let (captured, _guard) = capture();
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Pop {
            batch_id: "batch-1".to_owned(),
        })
        .await
        .expect("an oversized record is not an error");
    assert_eq!(outcome, BatchEnqueuedRefundResult::Popped(None));
    assert_eq!(captured.levels(), vec![Level::WARN]);
}

#[tokio::test]
async fn pop_reads_the_local_record_without_redis() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let reservation = redis_record(500, vec![key_scope()]);
    seed_record(&counters, &record_text(&reservation));
    let service = refund_service(None, Arc::clone(&counters));
    let (captured, _guard) = capture();
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Pop {
            batch_id: "batch-1".to_owned(),
        })
        .await
        .expect("the local pop answers");
    assert_eq!(
        outcome,
        BatchEnqueuedRefundResult::Popped(Some(reservation))
    );
    assert!(stored_tokens(&counters).is_none());
    assert!(captured.levels().is_empty());
}

#[tokio::test]
async fn pop_falls_back_to_local_when_the_script_fails() {
    let time = Arc::new(Mutex::new(0.0_f64));
    let counters = counters_at(&time);
    let reservation = redis_record(500, vec![key_scope()]);
    seed_record(&counters, &record_text(&reservation));
    let scripts = ScriptedScripts::new(
        Vec::new(),
        Vec::new(),
        vec![PopReply::Fail("down".to_owned())],
    );
    let service = refund_service(Some(scripts.clone()), Arc::clone(&counters));
    let (captured, _guard) = capture();
    let outcome = service
        .run(&BatchEnqueuedRefundInput::Pop {
            batch_id: "batch-1".to_owned(),
        })
        .await
        .expect("the pop falls back to the local record");
    assert_eq!(
        outcome,
        BatchEnqueuedRefundResult::Popped(Some(reservation))
    );
    assert_eq!(scripts.calls(), vec![pop_call()]);
    assert!(stored_tokens(&counters).is_none());
    assert_eq!(captured.levels(), vec![Level::WARN]);
}

#[test]
fn record_round_trips_the_reservation() {
    let reservation = BatchEnqueuedTokenReservation {
        tokens: 500,
        scopes: vec![key_scope()],
        backend: ReservationBackend::Memory,
        owner: "worker-a".to_owned(),
        reserved_at_monotonic: RESERVED_AT,
    };
    let derived = ReservationRecord::from(&reservation);
    assert_eq!(
        derived,
        record(
            500,
            ReservationBackend::Memory,
            "worker-a",
            vec![key_scope()]
        )
    );
    assert_eq!(derived.tokens, reservation.tokens);
    assert_eq!(derived.scopes, reservation.scopes);
    assert_eq!(derived.owner, reservation.owner);
}

#[test]
fn a_record_defaults_to_the_redis_backend() {
    let explicit: ReservationRecord = serde_json::from_str(
        r#"{"tokens":5,"scopes":[],"reserved_at_monotonic":1.5,"owner":"","backend":"redis"}"#,
    )
    .expect("the record decodes");
    assert_eq!(explicit.backend, ReservationBackend::Redis);
    let defaults: ReservationRecord =
        serde_json::from_str(r#"{"tokens":5,"scopes":[],"reserved_at_monotonic":1.5}"#)
            .expect("the defaults decode");
    assert_eq!(defaults.backend, ReservationBackend::Redis);
    assert_eq!(defaults.owner, "");
}

#[test]
fn status_set_matches_the_source() {
    assert_eq!(
        BATCH_ENQUEUED_REFUND_STATUSES,
        [
            "completed",
            "complete",
            "failed",
            "expired",
            "cancelled",
            "cancelling"
        ]
    );
}

#[test]
fn settings_default_to_the_source_ttl() {
    assert_eq!(BatchEnqueuedTokenSettings::default().ttl_seconds, TTL);
}
