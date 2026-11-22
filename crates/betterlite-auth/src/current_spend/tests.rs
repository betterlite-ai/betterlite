#![cfg(test)]

mod adapters;
mod postgres;
mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::Timestamp;
use rstest::rstest;
use rust_decimal::Decimal;
use tracing::Level;

use self::support::{
    Journal, JournalBatch, Script, ScriptFailure, ScriptedCache, ScriptedRedis, ScriptedRows,
    capture_events, entries, money, reseed_over, window_start,
};
use super::{
    BUDGET_UNVERIFIABLE_DETAIL, CounterReading, CurrentSpend, CurrentSpendError, SpendQuery,
    WindowDescriptors,
};
use crate::budget_counter::CounterEntityType;
use crate::spend_counter_batch::batch::CounterValue;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::values::{ReseedCacheValue, SpendValueError};

const KEY: &str = "spend:key:hashed";
const MARKER: &str = "spend_db_floor:spend:key:hashed";

const NO_WINDOW: WindowDescriptors<'static> = WindowDescriptors {
    entity_type: None,
    entity_id: None,
    duration: None,
    start: None,
};

struct Scene {
    cache: ScriptedCache,
    batch: JournalBatch,
    reseed: SpendCounterReseed<ScriptedRows>,
    lookups: Journal,
    journal: Journal,
}

impl Scene {
    fn new(
        redis: Option<ScriptedRedis>,
        memory: &[(&str, ReseedCacheValue)],
        rows: Option<ScriptedRows>,
    ) -> Self {
        let lookups = Journal::default();
        let journal = Journal::default();
        let stored = memory
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        let prepared = rows.map(|mut scripted| {
            scripted.lookups = Arc::clone(&lookups);
            scripted
        });
        Self {
            cache: ScriptedCache {
                redis,
                memory: Arc::new(Mutex::new(stored)),
                expiring: Mutex::new(Vec::new()),
                journal: Arc::clone(&journal),
            },
            batch: JournalBatch {
                answer: Mutex::new(None),
                recorded: Mutex::new(Vec::new()),
                journal: Arc::clone(&journal),
            },
            reseed: reseed_over(prepared),
            lookups,
            journal,
        }
    }

    fn service(&self, strict: bool) -> CurrentSpend<'_, ScriptedCache, JournalBatch, ScriptedRows> {
        CurrentSpend {
            cache: &self.cache,
            batch: &self.batch,
            reseed: &self.reseed,
            fail_closed_budget_enforcement: strict,
        }
    }

    async fn spend(
        &self,
        strict: bool,
        query: &SpendQuery<'_>,
    ) -> Result<Decimal, CurrentSpendError<ScriptFailure>> {
        self.service(strict).get_current_spend(query).await
    }
}

fn number(text: &str) -> ReseedCacheValue {
    ReseedCacheValue::Number(money(text))
}

fn redis(get: Option<ReseedCacheValue>, set_max: Script<Option<Decimal>>) -> ScriptedRedis {
    ScriptedRedis {
        get: Script::Answer(get),
        set_max,
    }
}

fn database(
    primary: Option<&str>,
    end_user: Option<&str>,
    window: Option<&str>,
    fails: bool,
) -> ScriptedRows {
    ScriptedRows {
        primary: primary.map(money),
        end_user: end_user.map(money),
        window: window.map(money),
        fails,
        lookups: Journal::default(),
    }
}

fn rows(primary: Option<&str>) -> ScriptedRows {
    database(primary, None, None, false)
}

fn query<'query>(
    fallback: &str,
    max_budget: Option<&str>,
    window: WindowDescriptors<'query>,
) -> SpendQuery<'query> {
    SpendQuery {
        counter_key: KEY,
        fallback_spend: money(fallback),
        max_budget: max_budget.map(money),
        window,
        fallback_authoritative: false,
    }
}

#[rstest]
#[case::amount(CounterValue::Amount(Decimal::from(3_i32)), Some(Decimal::from(3_i32)))]
#[case::missing(CounterValue::Missing, None)]
#[tokio::test]
async fn batch_answer_wins(#[case] batched: CounterValue, #[case] expected: Option<Decimal>) {
    let scene = Scene::new(
        Some(redis(Some(number("9")), Script::Fail)),
        &[(KEY, number("7"))],
        Some(rows(Some("8"))),
    );
    *scene.batch.answer.lock().unwrap() = Some(batched);
    let reading = scene
        .service(true)
        .read_spend_counter_cache_value(KEY)
        .await
        .unwrap();
    assert_eq!(
        reading,
        CounterReading {
            value: expected,
            authoritative: true
        }
    );
}

#[tokio::test]
async fn redis_clean_miss_never_local() {
    let scene = Scene::new(Some(redis(None, Script::Fail)), &[(KEY, number("7"))], None);
    let reading = scene
        .service(false)
        .read_spend_counter_cache_value(KEY)
        .await
        .unwrap();
    assert_eq!(
        reading,
        CounterReading {
            value: None,
            authoritative: true
        }
    );
}

#[rstest]
#[case::unreadable_value(Script::Answer(Some(ReseedCacheValue::Text("abc".to_owned()))), "get_current_spend: the Redis value is not a number, falling back to in-memory")]
#[case::read_failure(
    Script::Fail,
    "get_current_spend: Redis read failed, falling back to in-memory"
)]
#[tokio::test]
async fn redis_parse_error_debug_local_unverified(
    #[case] get: Script<Option<ReseedCacheValue>>,
    #[case] message: &str,
) {
    let redis = Some(ScriptedRedis {
        get,
        set_max: Script::Fail,
    });
    let scene = Scene::new(redis, &[(KEY, number("7"))], None);
    let (log, _guard) = capture_events();
    let reading = scene
        .service(false)
        .read_spend_counter_cache_value(KEY)
        .await
        .unwrap();
    assert_eq!(
        reading,
        CounterReading {
            value: Some(Decimal::from(7_i32)),
            authoritative: false
        }
    );
    let events = entries(&log.0);
    assert!(
        events.iter().any(|event| event.level == Level::DEBUG
            && event.message == message
            && event.counter_key.as_deref() == Some(KEY)),
        "{events:?}"
    );
}

#[tokio::test]
async fn redis_number_is_authoritative() {
    let scene = Scene::new(
        Some(redis(Some(number("4.5")), Script::Fail)),
        &[(KEY, number("7"))],
        None,
    );
    let reading = scene
        .service(false)
        .read_spend_counter_cache_value(KEY)
        .await
        .unwrap();
    assert_eq!(
        reading,
        CounterReading {
            value: Some(money("4.5")),
            authoritative: true
        }
    );
}

#[tokio::test]
async fn local_parse_error_propagates() {
    let scene = Scene::new(
        None,
        &[(KEY, ReseedCacheValue::Text("abc".to_owned()))],
        None,
    );
    let error = scene
        .service(false)
        .read_spend_counter_cache_value(KEY)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            CurrentSpendError::Value(SpendValueError::OutOfRange { .. })
        ),
        "{error}"
    );
    let absent = Scene::new(None, &[], None);
    let reading = absent
        .service(false)
        .read_spend_counter_cache_value(KEY)
        .await
        .unwrap();
    assert_eq!(
        reading,
        CounterReading {
            value: None,
            authoritative: false
        }
    );
}

#[rstest]
#[case::zero_reseed_verified(Some(rows(Some("0"))), Ok(Decimal::ZERO))]
#[case::no_database_fallback_unverified(None, Err(()))]
#[tokio::test]
async fn reseed_zero_verified_or_fallback_unverified(
    #[case] database: Option<ScriptedRows>,
    #[case] expected: Result<Decimal, ()>,
) {
    let scene = Scene::new(None, &[], database);
    let strict = scene.spend(true, &query("0", Some("10"), NO_WINDOW)).await;
    assert_eq!(strict.map_err(drop), expected);
    let lenient = Scene::new(None, &[], None)
        .spend(false, &query("2", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(lenient, Decimal::from(2_i32));
}

#[rstest]
#[case::authoritative(true, Ok(Decimal::from(2_i32)))]
#[case::default_unverified(false, Err(()))]
#[tokio::test]
async fn explicit_authoritative_fallback(
    #[case] fallback_authoritative: bool,
    #[case] expected: Result<Decimal, ()>,
) {
    let scene = Scene::new(None, &[], None);
    let request = SpendQuery {
        fallback_authoritative,
        ..query("2", Some("10"), NO_WINDOW)
    };
    assert_eq!(scene.spend(true, &request).await.map_err(drop), expected);
}

#[rstest]
#[case::no_ceiling(None, Ok(Decimal::from(5_i32)))]
#[case::over_ceiling(Some("4"), Ok(Decimal::from(5_i32)))]
#[case::at_ceiling(Some("5"), Ok(Decimal::from(5_i32)))]
#[case::under_ceiling(Some("5.01"), Err(()))]
#[tokio::test]
async fn no_ceiling_or_at_ceiling_returns_before_strict(
    #[case] max_budget: Option<&str>,
    #[case] expected: Result<Decimal, ()>,
) {
    let scene = Scene::new(None, &[(KEY, number("5"))], None);
    assert_eq!(
        scene
            .spend(true, &query("0", max_budget, NO_WINDOW))
            .await
            .map_err(drop),
        expected
    );
}

#[rstest]
#[case::nothing_signals("0", None, false, 0)]
#[case::equal_fallback("5", None, false, 0)]
#[case::higher_fallback("5.5", None, false, 1)]
#[case::window_start("0", Some(window_start()), false, 1)]
#[case::strict("0", None, true, 1)]
#[tokio::test]
async fn floor_trigger_truth_table(
    #[case] fallback: &str,
    #[case] start: Option<Timestamp>,
    #[case] strict: bool,
    #[case] reads: usize,
) {
    let scene = Scene::new(None, &[(KEY, number("5"))], Some(rows(Some("3"))));
    let window = WindowDescriptors { start, ..NO_WINDOW };
    let spend = scene
        .spend(strict, &query(fallback, Some("10"), window))
        .await
        .unwrap();
    assert_eq!(entries(&scene.lookups).len(), reads);
    assert_eq!(spend, Decimal::from(5_i32));
}

#[tokio::test]
async fn floor_lower_only_marks_verified() {
    let scene = Scene::new(None, &[(KEY, number("5"))], Some(rows(Some("3"))));
    let spend = scene
        .spend(true, &query("8", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(spend, Decimal::from(5_i32));
    assert_eq!(scene.cache.stored(KEY), Some(number("5")));
    assert!(entries(&scene.journal).is_empty());
    // a floor equal to the Redis counter only verifies it: nothing is forgotten or set-maxed
    let equal = Scene::new(
        Some(redis(Some(number("5")), Script::Answer(Some(money("5"))))),
        &[],
        Some(rows(Some("5"))),
    );
    let verified = equal
        .spend(true, &query("6", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(verified, Decimal::from(5_i32));
    assert!(entries(&equal.journal).is_empty());
    assert_eq!(equal.cache.stored(MARKER), Some(number("5")));
}

#[tokio::test]
async fn floor_higher_repairs_and_returns() {
    let scene = Scene::new(None, &[(KEY, number("5"))], Some(rows(Some("9"))));
    let spend = scene
        .spend(false, &query("6", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(spend, Decimal::from(9_i32));
    assert_eq!(scene.cache.stored(KEY), Some(number("9")));
}

#[tokio::test]
async fn missing_floor_higher_fallback_bypasses_strict() {
    let failing = Some(database(None, None, None, true));
    let scene = Scene::new(None, &[(KEY, number("5"))], failing);
    let spend = scene
        .spend(true, &query("7", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(spend, Decimal::from(7_i32));
    assert_eq!(scene.cache.stored(KEY), Some(number("5")));
}

#[tokio::test]
async fn strict_unverified_warning503_all_fields() {
    let scene = Scene::new(None, &[(KEY, number("5"))], None);
    let (log, _guard) = capture_events();
    let error = scene
        .spend(true, &query("0", Some("10"), NO_WINDOW))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, CurrentSpendError::Unverifiable { counter_key } if counter_key == KEY),
        "{error}"
    );
    let warnings: Vec<_> = entries(&log.0)
        .into_iter()
        .filter(|event| event.level == Level::WARN)
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(
        warnings
            .first()
            .and_then(|event| event.counter_key.as_deref()),
        Some(KEY)
    );
    assert_eq!(
        BUDGET_UNVERIFIABLE_DETAIL,
        "Budget enforcement unavailable: current spend could not be verified against Redis or the database, and fail_closed_budget_enforcement is enabled, so the request was rejected to avoid exceeding the configured budget. Retry shortly."
    );
    let lenient = scene
        .spend(false, &query("0", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(lenient, Decimal::from(5_i32));
}

#[tokio::test]
async fn floor_key_ttl5_no_negative_cache() {
    let scene = Scene::new(None, &[(KEY, number("5"))], Some(rows(Some("3"))));
    scene
        .spend(true, &query("0", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    scene
        .spend(true, &query("0", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(entries(&scene.lookups).len(), 1);
    assert_eq!(
        entries(&scene.cache.expiring),
        vec![(MARKER.to_owned(), Some(5))]
    );
    assert_eq!(scene.cache.stored(MARKER), Some(number("3")));

    let missing = Scene::new(None, &[(KEY, number("5"))], Some(rows(None)));
    missing
        .spend(false, &query("6", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    missing
        .spend(false, &query("6", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(entries(&missing.lookups).len(), 2);
    assert_eq!(missing.cache.stored(MARKER), None);
}

#[tokio::test]
async fn cached_floor_marker_used_and_unreadable_propagates() {
    let marked = Scene::new(
        None,
        &[
            (KEY, number("5")),
            (MARKER, ReseedCacheValue::Text("9.5".to_owned())),
        ],
        Some(rows(Some("3"))),
    );
    assert_eq!(
        marked
            .spend(false, &query("6", Some("10"), NO_WINDOW))
            .await
            .unwrap(),
        money("9.5")
    );
    assert!(entries(&marked.lookups).is_empty());
    let unreadable = Scene::new(
        None,
        &[(KEY, number("5")), (MARKER, ReseedCacheValue::Unreadable)],
        Some(rows(Some("3"))),
    );
    assert!(matches!(
        unreadable
            .spend(false, &query("6", Some("10"), NO_WINDOW))
            .await,
        Err(CurrentSpendError::Value(_))
    ));
}

#[tokio::test]
async fn enduser_no_window_fallback() {
    let end_user_key = "spend:end_user:customer";
    let window = WindowDescriptors {
        entity_type: Some(&CounterEntityType::Key),
        entity_id: Some("customer"),
        duration: Some("1d"),
        start: Some(window_start()),
    };
    let scene = Scene::new(
        None,
        &[(end_user_key, number("5"))],
        Some(database(Some("8"), None, Some("8"), false)),
    );
    let request = SpendQuery {
        counter_key: end_user_key,
        ..query("0", Some("10"), window)
    };
    assert_eq!(
        scene.spend(false, &request).await.unwrap(),
        Decimal::from(5_i32)
    );
    assert_eq!(
        entries(&scene.lookups),
        vec![r#"EndUser("customer")"#.to_owned()]
    );

    let found = Scene::new(
        None,
        &[(end_user_key, number("5"))],
        Some(database(None, Some("9"), None, false)),
    );
    assert_eq!(
        found.spend(false, &request).await.unwrap(),
        Decimal::from(9_i32)
    );
}

#[rstest]
#[case::complete(Some(&CounterEntityType::Key), Some("hashed"), Some(window_start()), None, 2)]
#[case::primary_row_wins(Some(&CounterEntityType::Key), Some("hashed"), Some(window_start()), Some("4"), 1)]
#[case::no_type(None, Some("hashed"), Some(window_start()), None, 1)]
#[case::no_id(Some(&CounterEntityType::Key), None, Some(window_start()), None, 1)]
#[case::no_start(Some(&CounterEntityType::Key), Some("hashed"), None, None, 1)]
#[tokio::test]
async fn primary_then_complete_window_descriptors(
    #[case] entity_type: Option<&'static CounterEntityType>,
    #[case] entity_id: Option<&'static str>,
    #[case] start: Option<Timestamp>,
    #[case] primary: Option<&str>,
    #[case] reads: usize,
) {
    let scene = Scene::new(
        Some(redis(Some(number("5")), Script::Answer(None))),
        &[],
        Some(database(primary, None, Some("2"), false)),
    );
    let window = WindowDescriptors {
        entity_type,
        entity_id,
        duration: Some("1d"),
        start,
    };
    scene
        .spend(true, &query("0", Some("10"), window))
        .await
        .unwrap();
    let lookups = entries(&scene.lookups);
    assert_eq!(lookups.len(), reads, "{lookups:?}");
    assert_eq!(
        lookups.first().map(String::as_str),
        Some(r#"Key("hashed")"#)
    );
    if reads == 2 {
        assert_eq!(
            lookups.get(1).map(String::as_str),
            Some("window Key hashed 1d")
        );
    }
}

#[rstest]
#[case::absent(&[], "9")]
#[case::lower(&[(KEY, ReseedCacheValue::Number(Decimal::from(3_i32)))], "9")]
#[case::unreadable(&[(KEY, ReseedCacheValue::Unreadable)], "9")]
#[case::equal(&[(KEY, ReseedCacheValue::Number(Decimal::from(9_i32)))], "9")]
#[case::higher(&[(KEY, ReseedCacheValue::Number(Decimal::from(12_i32)))], "12")]
#[tokio::test]
async fn local_repair_never_lowers(
    #[case] memory: &[(&str, ReseedCacheValue)],
    #[case] after: &str,
) {
    let scene = Scene::new(
        Some(redis(Some(number("5")), Script::Answer(None))),
        memory,
        Some(rows(Some("9"))),
    );
    let spend = scene
        .spend(false, &query("6", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(spend, Decimal::from(9_i32));
    assert_eq!(scene.cache.stored(KEY), Some(number(after)));
}

#[tokio::test]
async fn batch_forget_before_redis_setmax() {
    let scene = Scene::new(
        Some(redis(Some(number("5")), Script::Answer(Some(money("9"))))),
        &[],
        Some(rows(Some("9"))),
    );
    scene
        .spend(false, &query("6", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(
        entries(&scene.journal),
        vec![format!("forget {KEY}"), format!("set_max {KEY}")]
    );
    let local = Scene::new(None, &[(KEY, number("5"))], Some(rows(Some("9"))));
    local
        .spend(false, &query("6", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert!(entries(&local.journal).is_empty());
}

#[tokio::test]
async fn redis_repair_failure_keeps_local_and_returns_floor() {
    let scene = Scene::new(
        Some(redis(Some(number("5")), Script::Fail)),
        &[],
        Some(rows(Some("9"))),
    );
    let (log, _guard) = capture_events();
    let spend = scene
        .spend(false, &query("6", Some("10"), NO_WINDOW))
        .await
        .unwrap();
    assert_eq!(spend, Decimal::from(9_i32));
    assert_eq!(scene.cache.stored(KEY), Some(number("9")));
    assert!(entries(&scene.batch.recorded).is_empty());
    let events = entries(&log.0);
    assert!(
        events.iter().any(|event| event.level == Level::DEBUG
            && event.message == "Unable to repair stale spend counter in Redis"
            && event.counter_key.as_deref() == Some(KEY)),
        "{events:?}"
    );
}

#[rstest]
#[case::repaired(Some("11"), vec![(KEY.to_owned(), Decimal::from(11_i32))])]
#[case::no_result(None, Vec::new())]
#[tokio::test]
async fn nonnull_repair_records_batch(
    #[case] repaired: Option<&str>,
    #[case] recorded: Vec<(String, Decimal)>,
) {
    let scene = Scene::new(
        Some(redis(
            Some(number("5")),
            Script::Answer(repaired.map(money)),
        )),
        &[],
        Some(rows(Some("9"))),
    );
    assert_eq!(
        scene
            .spend(false, &query("6", Some("10"), NO_WINDOW))
            .await
            .unwrap(),
        Decimal::from(9_i32)
    );
    assert_eq!(entries(&scene.batch.recorded), recorded);
    assert_eq!(scene.batch.answer.lock().unwrap().as_ref(), None);
}
