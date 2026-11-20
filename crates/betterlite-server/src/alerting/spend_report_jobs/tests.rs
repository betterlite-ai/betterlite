#![cfg(test)]

use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::SlackAlertingArgs;
use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use rstest::rstest;
use rust_decimal::Decimal;
use uuid::Uuid;

use super::{
    FirstRunJitter, SpendJobRunner, SpendJobSchedule, SpendJobSettings, SpendJobTarget,
    SpendPodLock, plan_spend_jobs,
};

#[derive(Clone, Default)]
struct RecordingJitter {
    offset: u32,
    limits: Arc<Mutex<Vec<u32>>>,
}

impl FirstRunJitter for RecordingJitter {
    fn offset_seconds(&self, maximum: u32) -> u32 {
        self.limits.lock().unwrap().push(maximum);
        self.offset.min(maximum)
    }
}

#[derive(Clone, Default)]
struct FakeLock {
    answer: Option<bool>,
    asked: Arc<Mutex<Vec<(String, u64)>>>,
}

impl FakeLock {
    fn asked(&self) -> Vec<(String, u64)> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendPodLock for FakeLock {
    async fn acquire_lock(&self, job_id: &str, ttl_seconds: u64) -> Option<bool> {
        self.asked
            .lock()
            .unwrap()
            .push((job_id.to_owned(), ttl_seconds));
        self.answer
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum JobCall {
    Weekly(String),
    Monthly,
    User(bool),
}

#[derive(Clone, Default)]
struct FakeTarget {
    calls: Arc<Mutex<Vec<JobCall>>>,
}

impl FakeTarget {
    fn calls(&self) -> Vec<JobCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendJobTarget for FakeTarget {
    async fn weekly_spend_report(&self, time_range: &str) {
        self.calls
            .lock()
            .unwrap()
            .push(JobCall::Weekly(time_range.to_owned()));
    }

    async fn monthly_spend_report(&self) {
        self.calls.lock().unwrap().push(JobCall::Monthly);
    }

    async fn user_spend_alerts(&self, has_database: bool) {
        self.calls.lock().unwrap().push(JobCall::User(has_database));
    }
}

#[derive(Default)]
struct SharedLockCache {
    granted: bool,
    owner: Mutex<Option<String>>,
    ttls: Mutex<Vec<i64>>,
}

impl LockCache for SharedLockCache {
    fn get_owner(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        ready(Ok(self
            .owner
            .lock()
            .unwrap()
            .clone()
            .map(String::into_bytes)))
    }

    fn set_owner_if_absent(
        &self,
        _key: &str,
        owner: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        self.ttls.lock().unwrap().push(ttl_seconds);
        if self.granted {
            *self.owner.lock().unwrap() = Some(owner.to_owned());
        }
        ready(Ok(self.granted))
    }

    fn delete(&self, _key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        ready(Ok(0))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

fn pod_id() -> PodId {
    PodId::try_from(Uuid::new_v4()).unwrap()
}

fn args(user_spend_check_interval: u32) -> SlackAlertingArgs {
    SlackAlertingArgs {
        daily_report_frequency: 43_200,
        report_check_interval: 300,
        budget_alert_ttl: 86_400,
        outage_alert_ttl: 60,
        region_outage_alert_ttl: 60,
        minor_outage_alert_threshold: 5,
        major_outage_alert_threshold: 10,
        max_outage_alert_list_size: 10,
        log_to_console: false,
        daily_user_spend_threshold: None,
        monthly_user_spend_threshold: None,
        spend_anomaly_multiplier: Decimal::from_parts(3, 0, 0, false, 0),
        spend_anomaly_baseline_days: 7,
        spend_anomaly_min_spend: Decimal::from_parts(10, 0, 0, false, 0),
        user_spend_check_interval,
    }
}

fn job(id: &'static str, lock_ttl_seconds: u64) -> SpendJobSettings {
    SpendJobSettings {
        id,
        schedule: SpendJobSchedule::MonthlyOnFirstDay,
        lock_ttl_seconds,
        first_run_delay_seconds: None,
    }
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[case::default(Some("7d"))]
fn plan_uses_seven_days_by_default(#[case] frequency: Option<&str>) {
    let plan = plan_spend_jobs(frequency, &args(3600), &RecordingJitter::default()).unwrap();
    assert_eq!(plan.weekly.schedule, SpendJobSchedule::EveryDays(7));
    assert_eq!(plan.weekly.lock_ttl_seconds, 601_200);
    assert_eq!(plan.weekly.first_run_delay_seconds, Some(10));
}

#[rstest]
#[case::one_day("1d", 1, 82_800)]
#[case::two_days("2d", 2, 169_200)]
#[case::uppercase_unit("7D", 7, 601_200)]
fn plan_computes_the_weekly_lock_ttl(
    #[case] frequency: &str,
    #[case] days: u32,
    #[case] lock_ttl_seconds: u64,
) {
    let plan = plan_spend_jobs(Some(frequency), &args(3600), &RecordingJitter::default()).unwrap();
    assert_eq!(plan.weekly.schedule, SpendJobSchedule::EveryDays(days));
    assert_eq!(plan.weekly.lock_ttl_seconds, lock_ttl_seconds);
}

#[rstest]
#[case::missing_unit("7")]
#[case::wrong_unit("7m")]
#[case::zero_days("0d")]
#[case::negative("-1d")]
#[case::unit_only("d")]
fn plan_rejects_an_invalid_frequency(#[case] frequency: &str) {
    let error =
        plan_spend_jobs(Some(frequency), &args(3600), &RecordingJitter::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "spend_report_frequency must be a positive number of days, e.g., '1d', '7d'"
    );
}

#[test]
fn plan_returns_the_three_jobs_with_their_rules() {
    let jitter = RecordingJitter {
        offset: 42,
        limits: Arc::default(),
    };
    let plan = plan_spend_jobs(Some("7d"), &args(1800), &jitter).unwrap();
    assert_eq!(plan.weekly.id, "weekly_spend_report_job");
    assert_eq!(plan.weekly.first_run_delay_seconds, Some(52));
    assert_eq!(plan.monthly.id, "monthly_spend_report_job");
    assert_eq!(plan.monthly.schedule, SpendJobSchedule::MonthlyOnFirstDay);
    assert_eq!(plan.monthly.lock_ttl_seconds, 3_600);
    assert_eq!(plan.monthly.first_run_delay_seconds, None);
    assert_eq!(plan.user_spend.id, "user_spend_alerts_job");
    assert_eq!(
        plan.user_spend.schedule,
        SpendJobSchedule::EverySeconds(1800)
    );
    assert_eq!(plan.user_spend.lock_ttl_seconds, 1_740);
    assert_eq!(plan.user_spend.first_run_delay_seconds, Some(52));
    assert_eq!(jitter.limits.lock().unwrap().clone(), vec![300, 60]);
}

#[test]
fn plan_clamps_the_jitter_to_its_range() {
    let jitter = RecordingJitter {
        offset: 1_000,
        limits: Arc::default(),
    };
    let plan = plan_spend_jobs(Some("7d"), &args(3600), &jitter).unwrap();
    assert_eq!(plan.weekly.first_run_delay_seconds, Some(310));
    assert_eq!(plan.user_spend.first_run_delay_seconds, Some(70));
}

#[rstest]
#[case::interval(3600, 3_540)]
#[case::minimum_interval(60, 60)]
#[case::below_the_minimum(30, 60)]
fn plan_keeps_the_user_lock_ttl_at_least_sixty(
    #[case] interval: u32,
    #[case] lock_ttl_seconds: u64,
) {
    let plan = plan_spend_jobs(Some("7d"), &args(interval), &RecordingJitter::default()).unwrap();
    assert_eq!(plan.user_spend.lock_ttl_seconds, lock_ttl_seconds);
}

#[rstest]
#[case::acquired(Some(true))]
#[case::no_cache(None)]
#[tokio::test]
async fn run_weekly_runs_when_the_lock_is_true_or_none(#[case] answer: Option<bool>) {
    let target = FakeTarget::default();
    let runner = SpendJobRunner {
        lock: FakeLock {
            answer,
            asked: Arc::default(),
        },
        target: target.clone(),
        has_database: true,
    };
    runner
        .run_weekly(&job("weekly_spend_report_job", 601_200), "7d")
        .await;
    assert_eq!(target.calls(), vec![JobCall::Weekly("7d".to_owned())]);
}

#[tokio::test]
async fn run_weekly_skips_only_when_the_lock_is_false() {
    let lock = FakeLock {
        answer: Some(false),
        asked: Arc::default(),
    };
    let target = FakeTarget::default();
    let runner = SpendJobRunner {
        lock: lock.clone(),
        target: target.clone(),
        has_database: true,
    };
    runner
        .run_weekly(&job("weekly_spend_report_job", 601_200), "7d")
        .await;
    assert!(target.calls().is_empty());
    assert_eq!(
        lock.asked(),
        vec![("weekly_spend_report_job".to_owned(), 601_200)]
    );
}

#[tokio::test]
async fn run_monthly_acquires_the_monthly_job() {
    let lock = FakeLock {
        answer: Some(true),
        asked: Arc::default(),
    };
    let target = FakeTarget::default();
    let runner = SpendJobRunner {
        lock: lock.clone(),
        target: target.clone(),
        has_database: true,
    };
    runner
        .run_monthly(&job("monthly_spend_report_job", 3_600))
        .await;
    assert_eq!(target.calls(), vec![JobCall::Monthly]);
    assert_eq!(
        lock.asked(),
        vec![("monthly_spend_report_job".to_owned(), 3_600)]
    );
}

#[rstest]
#[case::with_database(true, JobCall::User(true))]
#[case::without_database(false, JobCall::User(false))]
#[tokio::test]
async fn run_user_spend_passes_the_database_flag(
    #[case] has_database: bool,
    #[case] call: JobCall,
) {
    let target = FakeTarget::default();
    let runner = SpendJobRunner {
        lock: FakeLock {
            answer: Some(true),
            asked: Arc::default(),
        },
        target: target.clone(),
        has_database,
    };
    runner
        .run_user_spend(&job("user_spend_alerts_job", 3_540))
        .await;
    assert_eq!(target.calls(), vec![call]);
}

#[tokio::test]
async fn the_pod_lock_adapter_passes_the_job_ttl() {
    let cache = Arc::new(SharedLockCache {
        granted: true,
        ..SharedLockCache::default()
    });
    let manager = PodLockManager::new(
        Some(Arc::clone(&cache)),
        30,
        Arc::new(NoPodLockEventSink),
        pod_id(),
    );
    let acquired = SpendPodLock::acquire_lock(&manager, "weekly_spend_report_job", 601_200).await;
    assert_eq!(acquired, Some(true));
    assert_eq!(cache.ttls.lock().unwrap().clone(), vec![601_200]);
}

#[tokio::test]
async fn the_pod_lock_adapter_refuses_a_reentrant_acquisition() {
    let holder = pod_id();
    let cache = Arc::new(SharedLockCache {
        granted: false,
        owner: Mutex::new(Some(holder.as_ref().to_owned())),
        ..SharedLockCache::default()
    });
    let manager = PodLockManager::new(
        Some(Arc::clone(&cache)),
        30,
        Arc::new(NoPodLockEventSink),
        holder,
    );
    let refused = SpendPodLock::acquire_lock(&manager, "weekly_spend_report_job", 601_200).await;
    assert_eq!(refused, Some(false));
    let reentrant = manager
        .acquire_lock("weekly_spend_report_job", Some(601_200), true)
        .await;
    assert_eq!(reentrant, Some(true));
    assert_eq!(cache.ttls.lock().unwrap().clone(), vec![601_200, 601_200]);
}

#[tokio::test]
async fn run_user_spend_skips_when_the_lock_is_false() {
    let lock = FakeLock {
        answer: Some(false),
        asked: Arc::default(),
    };
    let target = FakeTarget::default();
    let runner = SpendJobRunner {
        lock: lock.clone(),
        target: target.clone(),
        has_database: true,
    };
    runner
        .run_user_spend(&job("user_spend_alerts_job", 3_540))
        .await;
    assert!(target.calls().is_empty());
    assert_eq!(
        lock.asked(),
        vec![("user_spend_alerts_job".to_owned(), 3_540)]
    );
}
