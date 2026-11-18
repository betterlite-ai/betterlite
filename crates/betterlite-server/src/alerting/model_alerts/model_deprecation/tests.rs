#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::AlertLevel;
use betterlite_logging::alert_dispatch::destination::AlertDestination;
use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use chrono::{DateTime, NaiveDate, Utc};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;
use tokio_util::sync::CancellationToken;

use super::{
    AlertPodLock, AlertSleeper, CancellationSleeper, DEPRECATION_LOCK_ID, DEPRECATION_SENT_KEY,
    DeprecationRouterSource, ModelCatalogSource, ModelDeprecationAlertError,
    ModelDeprecationAlerts, deprecation_alert_level, deprecation_alert_metadata,
};
use crate::alerting::model_alerts::tests::support::{
    CacheWrite, FakeCache, FakeSender, Journal, LogCapture, SentAlert,
};
use crate::model_deprecation::catalog::{CatalogDeprecation, DeprecationCatalog};
use crate::model_deprecation::clock::UtcClock;
use crate::model_deprecation::types::{
    DeprecationStatus, ModelDeprecationInfo, ModelDeprecationResponse,
};

#[derive(Clone)]
struct FixedClock(DateTime<Utc>);

impl UtcClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LockCall {
    cronjob_id: String,
    ttl: Option<i64>,
    allow_reentrant: bool,
}

#[derive(Clone)]
struct FakePodLock {
    answer: Option<bool>,
    calls: Journal<LockCall>,
}

impl FakePodLock {
    fn returning(answer: Option<bool>) -> Self {
        Self {
            answer,
            calls: Journal::default(),
        }
    }
}

#[async_trait]
impl AlertPodLock for FakePodLock {
    async fn acquire_lock(
        &self,
        cronjob_id: &str,
        ttl: Option<i64>,
        allow_reentrant: bool,
    ) -> Option<bool> {
        self.calls.record(LockCall {
            cronjob_id: cronjob_id.to_owned(),
            ttl,
            allow_reentrant,
        });
        self.answer
    }
}

#[derive(Clone)]
struct EmptyCatalog;

impl DeprecationCatalog for EmptyCatalog {
    fn deprecation(&self, _model_key: &str) -> Option<CatalogDeprecation> {
        None
    }
}

#[derive(Clone)]
struct FakeCatalogSource {
    catalog: EmptyCatalog,
    calls: Journal<()>,
}

impl FakeCatalogSource {
    fn new() -> Self {
        Self {
            catalog: EmptyCatalog,
            calls: Journal::default(),
        }
    }
}

impl ModelCatalogSource for FakeCatalogSource {
    fn deprecation_catalog(&self) -> &dyn DeprecationCatalog {
        self.calls.record(());
        &self.catalog
    }
}

#[derive(Clone)]
struct FakeRouterSource {
    deployments: Option<Vec<DeploymentSpec>>,
    calls: Journal<()>,
}

impl FakeRouterSource {
    fn new(deployments: Option<Vec<DeploymentSpec>>) -> Self {
        Self {
            deployments,
            calls: Journal::default(),
        }
    }
}

impl DeprecationRouterSource for FakeRouterSource {
    fn deployments(&self) -> Option<Vec<DeploymentSpec>> {
        self.calls.record(());
        self.deployments.clone()
    }
}

#[derive(Clone)]
struct FakeSleeper {
    sleeps: Journal<u64>,
    answers: Arc<Mutex<VecDeque<bool>>>,
}

impl FakeSleeper {
    fn with_answers(answers: Vec<bool>) -> Self {
        Self {
            sleeps: Journal::default(),
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
        }
    }
}

#[async_trait]
impl AlertSleeper for FakeSleeper {
    async fn sleep(&self, seconds: u64) -> bool {
        self.sleeps.record(seconds);
        self.answers
            .lock()
            .expect("sleeper lock")
            .pop_front()
            .unwrap_or(false)
    }
}

type Service = ModelDeprecationAlerts<
    FakeCache,
    FakeSender,
    FixedClock,
    FakeSleeper,
    FakeRouterSource,
    FakeCatalogSource,
>;

fn now() -> DateTime<Utc> {
    "2026-05-10T12:00:00Z".parse().expect("a fixed instant")
}

fn deployment(name: &str, date: &str) -> DeploymentSpec {
    let mut model_info = IndexMap::new();
    model_info.insert("deprecation_date".to_owned(), Json::Text(date.to_owned()));
    DeploymentSpec {
        model_name: name.to_owned(),
        litellm_params: IndexMap::new(),
        model_info,
        extra: IndexMap::new(),
    }
}

fn deprecated_deployment() -> DeploymentSpec {
    deployment("old", "2026-05-01")
}

fn service() -> Service {
    ModelDeprecationAlerts {
        alerting: Some(vec![AlertDestination::Slack]),
        alert_types: vec![AlertType::ModelDeprecationWarnings],
        cache: FakeCache::default(),
        sender: FakeSender::default(),
        clock: FixedClock(now()),
        sleeper: FakeSleeper::with_answers(vec![false]),
        router_source: FakeRouterSource::new(Some(vec![deprecated_deployment()])),
        catalog_source: FakeCatalogSource::new(),
    }
}

fn bucket(count: usize, status: DeprecationStatus) -> Vec<ModelDeprecationInfo> {
    (0..count)
        .map(|index| ModelDeprecationInfo {
            model_name: format!("model-{index}"),
            litellm_model: None,
            deprecation_date: NaiveDate::from_ymd_opt(2026, 5, 1).expect("a valid date"),
            days_until_deprecation: 0,
            status,
            litellm_provider: None,
        })
        .collect()
}

fn snapshot(deprecated: usize, imminent: usize, upcoming: usize) -> ModelDeprecationResponse {
    ModelDeprecationResponse {
        deprecated: bucket(deprecated, DeprecationStatus::Deprecated),
        imminent: bucket(imminent, DeprecationStatus::Imminent),
        upcoming: bucket(upcoming, DeprecationStatus::Upcoming),
        warn_within_days: 30,
        checked_at: now(),
    }
}

fn sent_alerts(sender: &FakeSender) -> Vec<SentAlert> {
    sender.alerts.entries()
}

#[rstest]
#[case::enabled(Some(vec![AlertDestination::Slack]), vec![AlertType::ModelDeprecationWarnings], true)]
#[case::no_destinations(None, vec![AlertType::ModelDeprecationWarnings], false)]
#[case::type_missing(Some(vec![AlertDestination::Slack]), vec![AlertType::FallbackReports], false)]
fn enabled_needs_destinations_and_the_deprecation_type(
    #[case] alerting: Option<Vec<AlertDestination>>,
    #[case] alert_types: Vec<AlertType>,
    #[case] expected: bool,
) {
    let mut alerts = service();
    alerts.alerting = alerting;
    alerts.alert_types = alert_types;

    assert_eq!(alerts.deprecation_alerts_enabled(), expected);
}

#[tokio::test]
async fn a_disabled_send_collects_nothing() {
    let mut alerts = service();
    alerts.alerting = None;
    let sender = FakeSender::default();
    alerts.sender = sender.clone();

    let sent = alerts
        .send_model_deprecation_alert(Some(&[deprecated_deployment()]), None::<&FakePodLock>)
        .await;

    assert!(!sent.unwrap());
    assert!(alerts.catalog_source.calls.entries().is_empty());
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes.entries().is_empty());
}

#[tokio::test]
async fn a_missing_type_collects_nothing() {
    let mut alerts = service();
    alerts.alert_types = vec![AlertType::FallbackReports];

    let sent = alerts
        .send_model_deprecation_alert(Some(&[deprecated_deployment()]), None::<&FakePodLock>)
        .await;

    assert!(!sent.unwrap());
    assert!(alerts.catalog_source.calls.entries().is_empty());
}

#[tokio::test]
async fn no_message_returns_false_before_the_lock() {
    let lock = FakePodLock::returning(Some(true));
    let sender = FakeSender::default();
    let mut alerts = service();
    alerts.sender = sender.clone();

    let sent = alerts
        .send_model_deprecation_alert(Some(&[deployment("later", "2026-09-01")]), Some(&lock))
        .await;

    assert!(!sent.unwrap());
    assert!(lock.calls.entries().is_empty());
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes.entries().is_empty());
}

#[tokio::test]
async fn claims_the_lock_with_the_day_ttl_and_no_reentry() {
    let lock = FakePodLock::returning(Some(true));
    let mut alerts = service();

    let sent = alerts
        .send_model_deprecation_alert(Some(&[deprecated_deployment()]), Some(&lock))
        .await;

    assert!(sent.unwrap());
    assert_eq!(
        lock.calls.entries(),
        vec![LockCall {
            cronjob_id: DEPRECATION_LOCK_ID.to_owned(),
            ttl: Some(86_400),
            allow_reentrant: false,
        }]
    );
}

#[tokio::test]
async fn an_explicit_false_from_the_lock_refuses_the_send() {
    let lock = FakePodLock::returning(Some(false));
    let sender = FakeSender::default();
    let mut alerts = service();
    alerts.sender = sender.clone();

    let sent = alerts
        .send_model_deprecation_alert(Some(&[deprecated_deployment()]), Some(&lock))
        .await;

    assert!(!sent.unwrap());
    assert!(sender.alerts.entries().is_empty());
    assert!(alerts.cache.writes.entries().is_empty());
}

#[tokio::test]
async fn a_none_lock_answer_still_sends() {
    let lock = FakePodLock::returning(None);
    let sender = FakeSender::default();
    let mut alerts = service();
    alerts.sender = sender.clone();

    let sent = alerts
        .send_model_deprecation_alert(Some(&[deprecated_deployment()]), Some(&lock))
        .await;

    assert!(sent.unwrap());
    assert_eq!(sent_alerts(&sender).len(), 1);
}

#[tokio::test]
async fn no_pod_lock_still_sends() {
    let sender = FakeSender::default();
    let mut alerts = service();
    alerts.sender = sender.clone();

    let sent = alerts
        .send_model_deprecation_alert(Some(&[deprecated_deployment()]), None::<&FakePodLock>)
        .await;

    assert!(sent.unwrap());
    assert_eq!(sent_alerts(&sender).len(), 1);
}

#[rstest]
#[case::high(1, 1, 1, AlertLevel::High)]
#[case::medium(0, 1, 1, AlertLevel::Medium)]
fn the_level_follows_the_deprecated_bucket(
    #[case] deprecated: usize,
    #[case] imminent: usize,
    #[case] upcoming: usize,
    #[case] expected: AlertLevel,
) {
    let response = snapshot(deprecated, imminent, upcoming);

    assert_eq!(deprecation_alert_level(&response), expected);
}

#[test]
fn the_metadata_holds_the_three_counts_in_order() {
    let metadata = deprecation_alert_metadata(&snapshot(2, 3, 4));

    assert_eq!(
        metadata.keys().cloned().collect::<Vec<String>>(),
        vec![
            "deprecated_count".to_owned(),
            "imminent_count".to_owned(),
            "upcoming_count".to_owned()
        ]
    );
    assert_eq!(
        metadata.get("deprecated_count"),
        Some(&Json::Number(Number::from(2_usize)))
    );
    assert_eq!(
        metadata.get("imminent_count"),
        Some(&Json::Number(Number::from(3_usize)))
    );
    assert_eq!(
        metadata.get("upcoming_count"),
        Some(&Json::Number(Number::from(4_usize)))
    );
}

#[rstest]
#[case::deprecated_only(vec![deprecated_deployment()], AlertLevel::High, 1, 0, 0)]
#[case::imminent_only(vec![deployment("soon", "2026-05-20")], AlertLevel::Medium, 0, 1, 0)]
#[case::all_buckets(
    vec![
        deprecated_deployment(),
        deployment("soon", "2026-05-20"),
        deployment("later", "2026-09-01")
    ],
    AlertLevel::High,
    1,
    1,
    1
)]
#[tokio::test]
async fn the_alert_carries_the_level_and_the_counts(
    #[case] deployments: Vec<DeploymentSpec>,
    #[case] level: AlertLevel,
    #[case] deprecated: usize,
    #[case] imminent: usize,
    #[case] upcoming: usize,
) {
    let sender = FakeSender::default();
    let mut alerts = service();
    alerts.sender = sender.clone();

    let sent = alerts
        .send_model_deprecation_alert(Some(&deployments), None::<&FakePodLock>)
        .await;

    assert!(sent.unwrap());
    let sent = sent_alerts(&sender);
    let alert = sent.first().expect("one deprecation alert");
    assert_eq!(alert.level, level);
    assert_eq!(alert.alert_type, AlertType::ModelDeprecationWarnings);
    assert!(alert.user_info.is_none());
    assert_eq!(
        alert.metadata.get("deprecated_count"),
        Some(&Json::Number(Number::from(deprecated)))
    );
    assert_eq!(
        alert.metadata.get("imminent_count"),
        Some(&Json::Number(Number::from(imminent)))
    );
    assert_eq!(
        alert.metadata.get("upcoming_count"),
        Some(&Json::Number(Number::from(upcoming)))
    );
    assert!(alert.kwargs.is_empty());
}

#[tokio::test]
async fn the_stamp_is_written_after_the_send() {
    let mut alerts = service();

    let sent = alerts
        .send_model_deprecation_alert(Some(&[deprecated_deployment()]), None::<&FakePodLock>)
        .await;

    assert!(sent.unwrap());
    let seconds = now().timestamp();
    let value = num_traits::cast::<i64, f64>(seconds).expect("the instant is a float");
    let expected = Json::Number(Number::from_f64(value).expect("the instant is a number"));
    assert_eq!(
        alerts.cache.writes.entries(),
        vec![CacheWrite {
            key: DEPRECATION_SENT_KEY.to_owned(),
            value: expected,
            ttl_seconds: 86_400,
        }]
    );
}

#[tokio::test]
async fn a_failed_send_stamps_nothing() {
    let mut alerts = service();
    alerts.sender = FakeSender::failing();

    let sent = alerts
        .send_model_deprecation_alert(Some(&[deprecated_deployment()]), None::<&FakePodLock>)
        .await;

    assert!(matches!(sent, Err(ModelDeprecationAlertError::Alert(_))));
    assert!(alerts.cache.writes.entries().is_empty());
}

#[tokio::test]
async fn a_pass_returns_false_without_a_router() {
    let mut alerts = service();
    alerts.router_source = FakeRouterSource::new(None);

    let passed = alerts
        .run_deprecation_alert_pass(None, None::<&FakePodLock>)
        .await;

    assert!(!passed.unwrap());
    assert!(alerts.cache.writes.entries().is_empty());
}

#[tokio::test]
async fn a_pass_returns_false_when_the_alert_is_disabled() {
    let mut alerts = service();
    alerts.alerting = None;

    let passed = alerts
        .run_deprecation_alert_pass(Some(&[deprecated_deployment()]), None::<&FakePodLock>)
        .await;

    assert!(!passed.unwrap());
}

#[tokio::test]
async fn a_pass_returns_false_when_the_stamp_is_present() {
    let sender = FakeSender::default();
    let mut alerts = service();
    alerts.cache = FakeCache::seeded(DEPRECATION_SENT_KEY);
    alerts.sender = sender.clone();

    let passed = alerts
        .run_deprecation_alert_pass(Some(&[deprecated_deployment()]), None::<&FakePodLock>)
        .await;

    assert!(!passed.unwrap());
    assert!(sender.alerts.entries().is_empty());
}

#[tokio::test]
async fn a_pass_sends_and_stamps_when_clear() {
    let sender = FakeSender::default();
    let mut alerts = service();
    alerts.sender = sender.clone();

    let passed = alerts
        .run_deprecation_alert_pass(Some(&[deprecated_deployment()]), None::<&FakePodLock>)
        .await;

    assert!(passed.unwrap());
    assert_eq!(sent_alerts(&sender).len(), 1);
    assert_eq!(alerts.cache.writes.entries().len(), 1);
}

#[tokio::test]
async fn the_loop_polls_again_after_a_short_sleep() {
    let mut alerts = service();
    alerts.router_source = FakeRouterSource::new(Some(vec![deployment("later", "2026-09-01")]));
    alerts.sleeper = FakeSleeper::with_answers(vec![true, false]);

    alerts
        .run_deprecation_check_loop(None::<&FakePodLock>)
        .await;

    assert_eq!(alerts.sleeper.sleeps.entries(), vec![30, 30]);
    assert_eq!(alerts.router_source.calls.entries().len(), 2);
}

#[tokio::test]
async fn the_loop_logs_an_error_and_sleeps_a_day() {
    let log = LogCapture::new();
    let mut alerts = service();
    alerts.sender = FakeSender::failing();
    alerts.sleeper = FakeSleeper::with_answers(vec![false]);

    alerts
        .run_deprecation_check_loop(None::<&FakePodLock>)
        .await;

    assert_eq!(alerts.sleeper.sleeps.entries(), vec![86_400]);
    let text = log.text();
    assert!(text.contains("Error in model deprecation alert loop"));
    assert!(text.contains("the fake sender failed"));
}

#[tokio::test]
async fn the_loop_stops_on_the_cancelled_sleep() {
    let mut alerts = service();
    alerts.sleeper = FakeSleeper::with_answers(vec![false]);

    alerts
        .run_deprecation_check_loop(None::<&FakePodLock>)
        .await;

    assert_eq!(alerts.router_source.calls.entries().len(), 1);
    assert_eq!(alerts.sleeper.sleeps.entries(), vec![30]);
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_sleeper_reports_cancellation() {
    let sleeper = CancellationSleeper {
        cancellation: CancellationToken::new(),
    };
    sleeper.cancellation.cancel();

    assert!(!sleeper.sleep(30).await);
}

#[rstest]
#[case::poll_delay(30)]
#[case::error_backoff(86_400)]
#[tokio::test(start_paused = true)]
async fn an_uncancelled_sleeper_waits_the_delay(#[case] seconds: u64) {
    let sleeper = CancellationSleeper {
        cancellation: CancellationToken::new(),
    };
    let started = tokio::time::Instant::now();

    assert!(sleeper.sleep(seconds).await);
    assert_eq!(started.elapsed(), Duration::from_secs(seconds));
}
