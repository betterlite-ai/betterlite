#![cfg(test)]

use std::sync::Mutex;

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::decisions::Json;
use rand::SeedableRng as _;
use rand::rngs::StdRng;

use super::{RandomJitter, ReportJitter};
use crate::alerting::deployment_reports::DailyReportError;
use crate::alerting::deployment_reports::tests::support::{
    CacheWrite, FakeCache, FakeJitter, FakePodLock, FakeRouter, FakeSendError, FakeSender,
    FakeSleeper, LockCall, LogCapture, NOW_SECONDS, SENT_KEY, TestReports, deployment,
    float_metric, int_metric, reports, router_with,
};

const DUE_STAMP: f64 = 1_767_182_400.0_f64;
const ONE_SECOND_SHORT: f64 = 1_767_182_401.0_f64;
const FREQUENCY_DUE_STAMP: f64 = 1_767_225_540.0_f64;

struct Scheduled {
    service: TestReports,
    cache: FakeCache,
    sender: FakeSender,
}

fn one_router() -> FakeRouter {
    router_with(vec![deployment("d1", "model-a", None)])
}

fn due_scheduled() -> Scheduled {
    let cache = FakeCache::seeded(&[
        (SENT_KEY, float_metric(DUE_STAMP)),
        ("d1:failed_requests_daily_metrics", int_metric(3)),
    ]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();
    Scheduled {
        service,
        cache,
        sender,
    }
}

fn stamp_write() -> CacheWrite {
    CacheWrite {
        key: SENT_KEY.to_owned(),
        value: float_metric(NOW_SECONDS),
    }
}

#[tokio::test]
async fn the_first_pass_stamps_the_window_and_sends_nothing() {
    let cache = FakeCache::default();
    let sender = FakeSender::default();
    let lock = FakePodLock::returning(Some(true));
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();

    let sent = service
        .run_scheduler_helper(&one_router(), Some(&lock))
        .await;

    assert!(!sent.unwrap());
    assert_eq!(cache.writes.entries(), vec![stamp_write()]);
    assert!(sender.alerts.entries().is_empty());
    assert!(lock.calls.entries().is_empty());
}

#[tokio::test]
async fn a_pass_inside_the_interval_does_nothing() {
    let cache = FakeCache::seeded(&[
        (SENT_KEY, float_metric(ONE_SECOND_SHORT)),
        ("d1:failed_requests_daily_metrics", int_metric(3)),
    ]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();

    let sent = service
        .run_scheduler_helper(&one_router(), None::<&FakePodLock>)
        .await;

    assert!(!sent.unwrap());
    assert!(cache.writes.entries().is_empty());
    assert!(sender.alerts.entries().is_empty());
}

#[tokio::test]
async fn a_pass_at_the_interval_boundary_sends() {
    let scenario = due_scheduled();

    let sent = scenario
        .service
        .run_scheduler_helper(&one_router(), None::<&FakePodLock>)
        .await;

    assert!(sent.unwrap());
    assert_eq!(scenario.sender.alerts.entries().len(), 1);
    assert_eq!(scenario.cache.writes.entries(), vec![stamp_write()]);
}

#[tokio::test]
async fn the_pod_lock_is_asked_with_the_frequency_as_ttl() {
    let cache = FakeCache::seeded(&[
        (SENT_KEY, float_metric(FREQUENCY_DUE_STAMP)),
        ("d1:failed_requests_daily_metrics", int_metric(3)),
    ]);
    let sender = FakeSender::default();
    let lock = FakePodLock::returning(Some(true));
    let mut service = reports();
    service.cache = cache;
    service.sender = sender.clone();
    service.alerting_args.daily_report_frequency = 60;

    let sent = service
        .run_scheduler_helper(&one_router(), Some(&lock))
        .await;

    assert!(sent.unwrap());
    assert_eq!(sender.alerts.entries().len(), 1);
    assert_eq!(
        lock.calls.entries(),
        vec![LockCall {
            cronjob_id: "slack_daily_report".to_owned(),
            ttl: Some(60),
            allow_reentrant: false,
        }]
    );
}

#[tokio::test]
async fn an_explicit_false_lock_answer_refuses_the_report() {
    let scenario = due_scheduled();
    let lock = FakePodLock::returning(Some(false));

    let sent = scenario
        .service
        .run_scheduler_helper(&one_router(), Some(&lock))
        .await;

    assert!(!sent.unwrap());
    assert_eq!(lock.calls.entries().len(), 1);
    assert!(scenario.sender.alerts.entries().is_empty());
    assert!(scenario.cache.writes.entries().is_empty());
}

#[tokio::test]
async fn a_none_lock_answer_still_sends() {
    let scenario = due_scheduled();
    let lock = FakePodLock::returning(None);

    let sent = scenario
        .service
        .run_scheduler_helper(&one_router(), Some(&lock))
        .await;

    assert!(sent.unwrap());
    assert_eq!(scenario.sender.alerts.entries().len(), 1);
}

#[tokio::test]
async fn a_non_numeric_stamp_sends_nothing() {
    let cache = FakeCache::seeded(&[
        (SENT_KEY, Json::Text("yesterday".to_owned())),
        ("d1:failed_requests_daily_metrics", int_metric(3)),
    ]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();

    let sent = service
        .run_scheduler_helper(&one_router(), None::<&FakePodLock>)
        .await;

    assert!(!sent.unwrap());
    assert!(cache.writes.entries().is_empty());
    assert!(sender.alerts.entries().is_empty());
}

#[tokio::test]
async fn an_integer_stamp_counts_as_a_send_time() {
    let cache = FakeCache::seeded(&[
        (SENT_KEY, int_metric(1_767_182_400)),
        ("d1:failed_requests_daily_metrics", int_metric(3)),
    ]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache;
    service.sender = sender.clone();

    let sent = service
        .run_scheduler_helper(&one_router(), None::<&FakePodLock>)
        .await;

    assert!(sent.unwrap());
    assert_eq!(sender.alerts.entries().len(), 1);
}

#[tokio::test]
async fn a_report_with_no_metrics_still_stamps_the_window() {
    let cache = FakeCache::seeded(&[(SENT_KEY, float_metric(DUE_STAMP))]);
    let sender = FakeSender::default();
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = sender.clone();

    let sent = service
        .run_scheduler_helper(&one_router(), None::<&FakePodLock>)
        .await;

    assert!(sent.unwrap());
    assert!(sender.alerts.entries().is_empty());
    assert_eq!(cache.writes.entries(), vec![stamp_write()]);
}

#[tokio::test]
async fn a_failed_send_returns_the_error_and_leaves_the_stamp() {
    let cache = FakeCache::seeded(&[
        (SENT_KEY, float_metric(DUE_STAMP)),
        ("d1:failed_requests_daily_metrics", int_metric(3)),
    ]);
    let mut service = reports();
    service.cache = cache.clone();
    service.sender = FakeSender::failing();

    let sent = service
        .run_scheduler_helper(&one_router(), None::<&FakePodLock>)
        .await;

    assert!(matches!(sent, Err(DailyReportError::Alert(FakeSendError))));
    assert!(cache.writes.entries().is_empty());
    assert_eq!(cache.batches.entries().len(), 1);
}

#[tokio::test]
async fn the_disabled_scheduler_returns_before_reading_the_cache() {
    let cache = FakeCache::default();
    let sleeper = FakeSleeper::with_answers(vec![true]);
    let mut service = reports();
    service.alert_types = vec![AlertType::FallbackReports];
    service.cache = cache.clone();
    service.sleeper = sleeper.clone();

    service
        .run_scheduled_daily_report(Some(&one_router()), None::<&FakePodLock>)
        .await;

    assert!(cache.reads.entries().is_empty());
    assert!(sleeper.sleeps.entries().is_empty());
}

#[tokio::test]
async fn a_scheduler_without_a_router_returns_at_once() {
    let cache = FakeCache::default();
    let sleeper = FakeSleeper::with_answers(vec![true]);
    let mut service = reports();
    service.cache = cache.clone();
    service.sleeper = sleeper.clone();

    service
        .run_scheduled_daily_report(None::<&FakeRouter>, None::<&FakePodLock>)
        .await;

    assert!(cache.reads.entries().is_empty());
    assert!(sleeper.sleeps.entries().is_empty());
}

#[tokio::test]
async fn the_loop_sleeps_a_jittered_interval_between_passes() {
    let cache = FakeCache::default();
    let sleeper = FakeSleeper::with_answers(vec![true, false]);
    let jitter = FakeJitter::with_answers(vec![297, 298]);
    let mut service = reports();
    service.cache = cache.clone();
    service.sleeper = sleeper.clone();
    service.jitter = jitter.clone();

    service
        .run_scheduled_daily_report(Some(&one_router()), None::<&FakePodLock>)
        .await;

    assert_eq!(jitter.bounds.entries(), vec![(297, 303), (297, 303)]);
    assert_eq!(sleeper.sleeps.entries(), vec![297, 298]);
    assert_eq!(cache.reads.entries().len(), 2);
}

#[tokio::test]
async fn the_loop_jitters_around_the_configured_check_interval() {
    let sleeper = FakeSleeper::with_answers(vec![false]);
    let jitter = FakeJitter::with_answers(vec![60]);
    let mut service = reports();
    service.alerting_args.report_check_interval = 60;
    service.sleeper = sleeper.clone();
    service.jitter = jitter.clone();

    service
        .run_scheduled_daily_report(Some(&one_router()), None::<&FakePodLock>)
        .await;

    assert_eq!(jitter.bounds.entries(), vec![(57, 63)]);
    assert_eq!(sleeper.sleeps.entries(), vec![60]);
}

#[tokio::test]
async fn a_negative_jitter_sleeps_zero() {
    let sleeper = FakeSleeper::with_answers(vec![false]);
    let jitter = FakeJitter::with_answers(vec![-1]);
    let mut service = reports();
    service.alerting_args.report_check_interval = 2;
    service.sleeper = sleeper.clone();
    service.jitter = jitter.clone();

    service
        .run_scheduled_daily_report(Some(&one_router()), None::<&FakePodLock>)
        .await;

    assert_eq!(jitter.bounds.entries(), vec![(-1, 5)]);
    assert_eq!(sleeper.sleeps.entries(), vec![0]);
}

#[tokio::test]
async fn the_loop_logs_a_failed_pass_and_keeps_polling() {
    let log = LogCapture::new();
    let cache = FakeCache::seeded(&[
        (SENT_KEY, float_metric(DUE_STAMP)),
        ("d1:failed_requests_daily_metrics", int_metric(3)),
    ]);
    let sleeper = FakeSleeper::with_answers(vec![true, false]);
    let mut service = reports();
    service.cache = cache;
    service.sender = FakeSender::failing();
    service.sleeper = sleeper.clone();
    service.jitter = FakeJitter::with_answers(vec![297, 297]);

    service
        .run_scheduled_daily_report(Some(&one_router()), None::<&FakePodLock>)
        .await;

    assert_eq!(sleeper.sleeps.entries(), vec![297, 297]);
    let text = log.text();
    assert!(text.contains("Error in the daily report scheduler"));
    assert!(text.contains("the fake sender failed"));
}

#[tokio::test]
async fn the_loop_stops_on_the_cancelled_sleep() {
    let cache = FakeCache::default();
    let sleeper = FakeSleeper::with_answers(vec![false]);
    let mut service = reports();
    service.cache = cache.clone();
    service.sleeper = sleeper.clone();

    service
        .run_scheduled_daily_report(Some(&one_router()), None::<&FakePodLock>)
        .await;

    assert_eq!(sleeper.sleeps.entries().len(), 1);
    assert_eq!(cache.reads.entries().len(), 1);
}

#[test]
fn the_random_jitter_stays_inside_its_bounds() {
    let jitter = RandomJitter {
        random: Mutex::new(StdRng::seed_from_u64(7)),
    };
    let sample: Vec<i64> = (0_u8..64).map(|_| jitter.between(297, 303)).collect();

    assert!(sample.iter().all(|value| (297..=303).contains(value)));
    assert!(sample.iter().any(|value| *value != 297));
}
