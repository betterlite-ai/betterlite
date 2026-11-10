#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::alert_settings::types::{AlertType, AlertTypeConfig};
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use chrono::{DateTime, FixedOffset, TimeZone as _};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use tokio::task::{Id, yield_now};
use tokio::time::{advance, timeout};
use tokio_util::task::TaskTracker;

use super::super::tests::{FakeDelivery, Reply, TextPayload, alert, capture_logs};
use super::{AlertDelivery, FlushTask, WEBHOOK_URL_ENV, WebhookAlertError};
use crate::alert_digest::{Clock, DigestAlert, DigestBuckets, DigestState};
use crate::batch::logger::{BatchQueue, BatchSettings};

#[derive(Debug)]
struct FakeClock {
    now: Mutex<DateTime<FixedOffset>>,
}

impl FakeClock {
    fn new(now: DateTime<FixedOffset>) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    fn set(&self, now: DateTime<FixedOffset>) {
        *self.now.lock().unwrap() = now;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<FixedOffset> {
        *self.now.lock().unwrap()
    }
}

fn time(hour: u32, minute: u32, second: u32) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(0)
        .expect("UTC is a valid offset")
        .with_ymd_and_hms(2026, 1, 1, hour, minute, second)
        .single()
        .expect("the test time is unambiguous")
}

struct Fixture {
    delivery: Arc<AlertDelivery<FakeDelivery>>,
    clock: Arc<FakeClock>,
}

fn build<R>(
    environment: &[(&str, &str)],
    config: BTreeMap<AlertType, AlertTypeConfig>,
    log_to_console: bool,
    replies: R,
) -> Fixture
where
    R: IntoIterator<Item = Reply>,
{
    let clock = Arc::new(FakeClock::new(time(12, 0, 0)));
    let concrete: Arc<FakeClock> = Arc::clone(&clock);
    let clock_trait: Arc<dyn Clock> = concrete;
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let resolved: Arc<dyn EnvironmentLookup + Send + Sync> = Arc::new(lookup);
    let delivery = Arc::new(AlertDelivery {
        queue: BatchQueue::new(BatchSettings::default()),
        digests: DigestBuckets {
            state: Mutex::new(DigestState {
                config,
                entries: IndexMap::new(),
            }),
            clock: clock_trait,
        },
        delivery: FakeDelivery::scripted(replies),
        log_to_console,
        environment: resolved,
        flush_task: FlushTask::default(),
    });
    Fixture { delivery, clock }
}

fn fixture<R>(environment: &[(&str, &str)], replies: R) -> Fixture
where
    R: IntoIterator<Item = Reply>,
{
    build(environment, BTreeMap::new(), false, replies)
}

fn console_fixture<R>(replies: R) -> Fixture
where
    R: IntoIterator<Item = Reply>,
{
    build(&[], BTreeMap::new(), true, replies)
}

fn digest_fixture<R>(environment: &[(&str, &str)], interval: u32, replies: R) -> Fixture
where
    R: IntoIterator<Item = Reply>,
{
    let config = BTreeMap::from([(
        AlertType::BudgetAlerts,
        AlertTypeConfig {
            digest: true,
            digest_interval: interval,
        },
    )]);
    build(environment, config, false, replies)
}

async fn settle(delivery: &AlertDelivery<FakeDelivery>, expected: usize) {
    let mut attempts = 0_u32;
    while delivery.delivery.request_count() < expected && attempts < 32 {
        yield_now().await;
        attempts = attempts.saturating_add(1);
    }
}

fn task_id(delivery: &AlertDelivery<FakeDelivery>) -> Id {
    delivery
        .flush_task
        .task
        .lock()
        .unwrap()
        .as_ref()
        .expect("the flush task is running")
        .id()
}

// A task that ignores the close must fail here instead of hanging until the mutants timeout
async fn stop(tasks: &TaskTracker) {
    tasks.close();
    advance(Duration::from_secs(5)).await;
    timeout(Duration::from_secs(15), tasks.wait())
        .await
        .expect("the flush task stops after the tracker closes");
}

#[tokio::test(start_paused = true)]
async fn the_periodic_flush_task_starts_only_when_ensure_started_is_called() {
    let fixture = fixture(&[], []);
    assert!(fixture.delivery.flush_task.task.lock().unwrap().is_none());
    let tasks = TaskTracker::new();
    fixture.delivery.ensure_started(&tasks);
    assert!(
        fixture
            .delivery
            .flush_task
            .task
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    );
    stop(&tasks).await;
}

#[tokio::test(start_paused = true)]
async fn ensure_started_is_idempotent_while_the_task_runs() {
    let fixture = fixture(&[], []);
    let tasks = TaskTracker::new();
    fixture.delivery.ensure_started(&tasks);
    let first = task_id(&fixture.delivery);
    fixture.delivery.ensure_started(&tasks);
    assert_eq!(task_id(&fixture.delivery), first);
    stop(&tasks).await;
}

#[tokio::test(start_paused = true)]
async fn ensure_started_replaces_a_finished_task() {
    let fixture = fixture(&[], []);
    let first_tasks = TaskTracker::new();
    fixture.delivery.ensure_started(&first_tasks);
    let first = task_id(&fixture.delivery);
    stop(&first_tasks).await;
    let second_tasks = TaskTracker::new();
    fixture.delivery.ensure_started(&second_tasks);
    assert_ne!(task_id(&fixture.delivery), first);
    stop(&second_tasks).await;
}

#[tokio::test(start_paused = true)]
async fn periodic_flush_drains_expired_digests_before_flushing_the_queue() {
    let fixture = digest_fixture(
        &[("PROXY_BASE_URL", "https://proxy.example")],
        1,
        [Reply::Status(200), Reply::Status(200)],
    );
    let _queued = fixture.delivery.queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "queued alert",
        None,
    ));
    fixture.delivery.digests.record(DigestAlert {
        alert_type: AlertType::BudgetAlerts,
        request_model: None,
        api_base: None,
        message: "digested alert",
        level: "High",
        webhook_url: WebhookUrl::Single(SecretString::from("https://hooks/digest")),
    });
    fixture.clock.set(time(12, 0, 2));
    let tasks = TaskTracker::new();
    fixture.delivery.ensure_started(&tasks);
    yield_now().await;
    assert_eq!(fixture.delivery.delivery.request_count(), 0);
    advance(Duration::from_secs(5)).await;
    settle(&fixture.delivery, 2).await;
    let requests = fixture.delivery.delivery.take_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url, "https://hooks/a");
    assert_eq!(requests[1].url, "https://hooks/digest");
    let digest_text = requests[1].body_as::<TextPayload>().text;
    assert!(digest_text.contains("Alert type: `budget_alerts` (Digest)"));
    assert!(digest_text.contains("Message: digested alert"));
    assert!(digest_text.contains("Proxy URL: `https://proxy.example`"));
    stop(&tasks).await;
}

#[tokio::test(start_paused = true)]
async fn periodic_flush_keeps_running_on_the_configured_interval() {
    let fixture = fixture(&[], [Reply::Status(200), Reply::Status(200)]);
    let tasks = TaskTracker::new();
    fixture.delivery.ensure_started(&tasks);
    yield_now().await;
    let _first = fixture.delivery.queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "first",
        None,
    ));
    advance(Duration::from_secs(5)).await;
    settle(&fixture.delivery, 1).await;
    assert_eq!(fixture.delivery.delivery.take_requests().len(), 1);
    let _second = fixture.delivery.queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "second",
        None,
    ));
    advance(Duration::from_secs(5)).await;
    settle(&fixture.delivery, 1).await;
    assert_eq!(fixture.delivery.delivery.take_requests().len(), 1);
    stop(&tasks).await;
}

#[tokio::test]
async fn a_delivery_with_console_logging_on_warns_about_the_flushed_payload() {
    let (logs, _guard) = capture_logs();
    let fixture = console_fixture([Reply::Status(200)]);
    let _queued = fixture.delivery.queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "hello",
        None,
    ));
    let _report = fixture.delivery.async_send_batch().await;
    let contents = logs.contents();
    assert!(contents.contains("WARN"));
    assert!(contents.contains("alert payload"));
    assert!(contents.contains("hello"));
}

#[derive(serde::Serialize)]
struct BudgetEvent {
    event: &'static str,
    spend: u32,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
struct RecordedBudgetEvent {
    event: String,
    spend: u32,
}

fn event() -> BudgetEvent {
    BudgetEvent {
        event: "budget_crossed",
        spend: 12,
    }
}

#[tokio::test]
async fn send_webhook_alert_posts_the_event_as_json_to_the_webhook_url() {
    let fixture = fixture(
        &[(WEBHOOK_URL_ENV, "https://hooks.example/webhook")],
        [Reply::Status(200)],
    );
    assert!(fixture.delivery.send_webhook_alert(&event()).await.unwrap());
    let requests = fixture.delivery.delivery.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "https://hooks.example/webhook");
    assert_eq!(
        requests[0].headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(
        requests[0].body_as::<RecordedBudgetEvent>(),
        RecordedBudgetEvent {
            event: "budget_crossed".to_owned(),
            spend: 12,
        }
    );
    assert_eq!(requests[0].deadline, Duration::from_secs(600));
}

#[rstest]
#[case::unset(&[])]
#[case::empty(&[(WEBHOOK_URL_ENV, "")])]
#[tokio::test]
async fn send_webhook_alert_fails_when_webhook_url_is_missing(
    #[case] environment: &[(&str, &str)],
) {
    let fixture = fixture(environment, []);
    let error = fixture
        .delivery
        .send_webhook_alert(&event())
        .await
        .expect_err("a missing WEBHOOK_URL is an error");
    assert!(matches!(error, WebhookAlertError::MissingUrl));
    assert_eq!(fixture.delivery.delivery.request_count(), 0);
}

#[rstest]
#[case::ok(200, true)]
#[case::accepted(202, false)]
#[case::no_content(204, false)]
#[case::redirect(302, false)]
#[tokio::test]
async fn send_webhook_alert_returns_true_only_on_status_200(
    #[case] status: u16,
    #[case] delivered: bool,
) {
    let fixture = fixture(
        &[(WEBHOOK_URL_ENV, "https://hooks.example/webhook")],
        [Reply::Status(status)],
    );
    assert_eq!(
        fixture.delivery.send_webhook_alert(&event()).await.unwrap(),
        delivered
    );
}

#[tokio::test]
async fn a_non_200_success_or_redirect_is_logged_at_error_and_returns_false() {
    let (logs, _guard) = capture_logs();
    let fixture = fixture(
        &[(WEBHOOK_URL_ENV, "https://hooks.example/webhook")],
        [Reply::Status(302)],
    );
    assert!(!fixture.delivery.send_webhook_alert(&event()).await.unwrap());
    let contents = logs.contents();
    assert!(contents.contains("ERROR"));
    assert!(contents.contains("the webhook alert was rejected"));
    assert!(contents.contains("rejected"));
}

#[rstest]
#[case::rate_limited(429)]
#[case::server_error(500)]
#[tokio::test]
async fn a_client_or_server_error_status_is_returned_as_an_error(#[case] status: u16) {
    let fixture = fixture(
        &[(WEBHOOK_URL_ENV, "https://hooks.example/webhook")],
        [Reply::Status(status)],
    );
    let error = fixture
        .delivery
        .send_webhook_alert(&event())
        .await
        .expect_err("a 4xx or 5xx answer is an error, not a silent false");
    match error {
        WebhookAlertError::Status { status: rejected } => assert_eq!(rejected, status),
        other @ (WebhookAlertError::MissingUrl
        | WebhookAlertError::Url(_)
        | WebhookAlertError::Encode(_)
        | WebhookAlertError::Transport(_)) => panic!("expected a status error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_transport_failure_is_returned_from_send_webhook_alert() {
    let fixture = fixture(
        &[(WEBHOOK_URL_ENV, "https://hooks.example/webhook")],
        [Reply::Transport],
    );
    let error = fixture
        .delivery
        .send_webhook_alert(&event())
        .await
        .expect_err("a transport failure is an error");
    assert!(matches!(error, WebhookAlertError::Transport(_)));
}

#[tokio::test]
async fn an_unparsable_webhook_url_is_returned_from_send_webhook_alert() {
    let fixture = fixture(&[(WEBHOOK_URL_ENV, "not a url")], []);
    let error = fixture
        .delivery
        .send_webhook_alert(&event())
        .await
        .expect_err("an invalid WEBHOOK_URL is an error");
    assert!(matches!(error, WebhookAlertError::Url(_)));
    assert_eq!(fixture.delivery.delivery.request_count(), 0);
}
