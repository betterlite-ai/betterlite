#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, poll_fn};
use std::io::{Result as IoResult, Write};
use std::mem;
use std::pin::pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::Duration;

use betterlite_config::sections::alert_settings::types::AlertType;
use http::HeaderMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::DeserializeOwned;
use tokio::sync::{Barrier, oneshot};
use tokio::time::timeout;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::{send_to_webhook, squash_payloads};
use crate::alert_settings::records::{AlertQueueItem, AlertText};
use crate::batch::logger::{BatchQueue, BatchSettings};
use crate::http_analytics_loggers::delivery::{Delivery, PostRequest, PostResponse};

#[derive(Clone, Default)]
pub(super) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(super) fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    let guard = set_default(subscriber);
    (buffer, guard)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reply {
    Status(u16),
    Transport,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake transport failed")]
pub(super) struct FakeError;

pub(super) struct RecordedPost {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) deadline: Duration,
}

impl RecordedPost {
    pub(super) fn body_as<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).expect("the recorded body matches the test wire model")
    }
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
pub(super) struct TextPayload {
    pub(super) text: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TeamsWireMessage {
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) attachments: Vec<TeamsWireAttachment>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TeamsWireAttachment {
    #[serde(rename = "contentType")]
    pub(super) content_type: String,
    pub(super) content: TeamsWireCard,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TeamsWireCard {
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) version: String,
    pub(super) body: Vec<TeamsWireTextBlock>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TeamsWireTextBlock {
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) text: String,
    pub(super) wrap: bool,
}

#[derive(Default)]
pub(super) struct FakeDelivery {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<RecordedPost>>,
    barrier: Mutex<Option<Arc<Barrier>>>,
    release: Mutex<Option<oneshot::Receiver<()>>>,
}

impl FakeDelivery {
    pub(super) fn scripted<R>(replies: R) -> Self
    where
        R: IntoIterator<Item = Reply>,
    {
        Self {
            replies: Mutex::new(replies.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
            barrier: Mutex::new(None),
            release: Mutex::new(None),
        }
    }

    pub(super) fn gated(requests: usize) -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
            barrier: Mutex::new(Some(Arc::new(Barrier::new(requests)))),
            release: Mutex::new(None),
        }
    }

    pub(super) fn with_release(release: oneshot::Receiver<()>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
            barrier: Mutex::new(None),
            release: Mutex::new(Some(release)),
        }
    }

    pub(super) fn take_requests(&self) -> Vec<RecordedPost> {
        mem::take(&mut *self.requests.lock().unwrap())
    }

    pub(super) fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Delivery for FakeDelivery {
    type Error = FakeError;

    async fn post(&self, request: PostRequest) -> Result<PostResponse, FakeError> {
        self.requests.lock().unwrap().push(RecordedPost {
            url: request.url.to_string(),
            headers: request.headers,
            body: request.body,
            deadline: request.deadline,
        });
        let gated = self.barrier.lock().unwrap().clone();
        if let Some(barrier) = gated {
            barrier.wait().await;
        }
        let release = self.release.lock().unwrap().take();
        if let Some(receiver) = release {
            let _released = receiver.await;
        }
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Reply::Status(200));
        match reply {
            Reply::Status(status) => Ok(PostResponse {
                status,
                body: b"rejected".to_vec(),
            }),
            Reply::Transport => Err(FakeError),
        }
    }
}

pub(super) fn alert(
    url: &str,
    alert_type: AlertType,
    text: &str,
    format: Option<&str>,
) -> AlertQueueItem {
    AlertQueueItem {
        url: SecretString::from(url),
        headers: BTreeMap::from([("Content-type".to_owned(), "application/json".to_owned())]),
        payload: AlertText {
            text: text.to_owned(),
        },
        alert_type,
        format: format.map(str::to_owned),
    }
}

fn with_header(mut item: AlertQueueItem, name: &str, value: &str) -> AlertQueueItem {
    item.headers.insert(name.to_owned(), value.to_owned());
    item
}

pub(super) fn queue() -> BatchQueue<AlertQueueItem> {
    BatchQueue::new(BatchSettings::default())
}

#[test]
fn identical_alerts_squash_into_one_group_with_their_count() {
    let alerts = vec![
        alert(
            "https://hooks/a",
            AlertType::BudgetAlerts,
            "budget reached",
            None,
        ),
        alert(
            "https://hooks/b",
            AlertType::BudgetAlerts,
            "budget reached",
            None,
        ),
        alert(
            "https://hooks/a",
            AlertType::BudgetAlerts,
            "budget reached",
            None,
        ),
    ];
    let squashed = squash_payloads(&alerts);
    assert_eq!(squashed.len(), 2);
    assert_eq!(squashed[0].count, 2);
    assert_eq!(squashed[0].item.url.expose_secret(), "https://hooks/a");
    assert_eq!(squashed[1].count, 1);
    assert_eq!(squashed[1].item.url.expose_secret(), "https://hooks/b");
}

#[test]
fn the_first_item_of_a_squash_group_keeps_its_headers() {
    let first = with_header(
        alert(
            "https://hooks/a",
            AlertType::BudgetAlerts,
            "budget reached",
            None,
        ),
        "x-first",
        "one",
    );
    let second = with_header(
        alert(
            "https://hooks/a",
            AlertType::BudgetAlerts,
            "budget reached",
            None,
        ),
        "x-first",
        "two",
    );
    let squashed = squash_payloads(&[first, second]);
    assert_eq!(squashed.len(), 1);
    assert_eq!(squashed[0].count, 2);
    assert_eq!(
        squashed[0].item.headers.get("x-first").map(String::as_str),
        Some("one")
    );
}

#[rstest]
#[case::url("https://hooks/b", AlertType::BudgetAlerts, "same text")]
#[case::alert_type("https://hooks/a", AlertType::OutageAlerts, "same text")]
#[case::text("https://hooks/a", AlertType::BudgetAlerts, "other text")]
fn a_different_field_keeps_an_alert_out_of_the_group(
    #[case] url: &str,
    #[case] alert_type: AlertType,
    #[case] text: &str,
) {
    let first = alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "same text",
        None,
    );
    let other = alert(url, alert_type, text, None);
    assert_eq!(squash_payloads(&[first, other]).len(), 2);
}

#[test]
fn teams_and_slack_alerts_squash_together_because_format_is_not_in_the_key() {
    let teams = alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "same text",
        Some("ms_teams"),
    );
    let slack = alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "same text",
        None,
    );
    let squashed = squash_payloads(&[teams, slack]);
    assert_eq!(squashed.len(), 1);
    assert_eq!(squashed[0].count, 2);
    assert_eq!(squashed[0].item.format.as_deref(), Some("ms_teams"));
}

#[rstest]
#[case::ok(200, true)]
#[case::created(201, false)]
#[case::no_content(204, false)]
#[case::redirect(302, false)]
#[case::bad_request(400, false)]
#[case::server_error(500, false)]
#[tokio::test]
async fn only_http_200_counts_as_delivered(#[case] status: u16, #[case] delivered: bool) {
    let delivery = FakeDelivery::scripted([Reply::Status(status)]);
    let alert = alert("https://hooks/a", AlertType::BudgetAlerts, "hello", None);
    assert_eq!(
        send_to_webhook(&delivery, false, &alert, 1).await,
        delivered
    );
    assert_eq!(delivery.request_count(), 1);
}

#[tokio::test]
async fn a_rejected_status_is_logged_at_debug_with_the_response_body() {
    let (logs, _guard) = capture_logs();
    let delivery = FakeDelivery::scripted([Reply::Status(204)]);
    let alert = alert("https://hooks/a", AlertType::BudgetAlerts, "hello", None);
    assert!(!send_to_webhook(&delivery, false, &alert, 1).await);
    let contents = logs.contents();
    assert!(contents.contains("DEBUG"));
    assert!(contents.contains("the webhook answered with a status other than 200"));
    assert!(contents.contains("rejected"));
    assert!(!contents.contains("https://hooks/a"));
}

#[tokio::test]
async fn a_transport_error_is_caught_and_logged_at_debug() {
    let (logs, _guard) = capture_logs();
    let delivery = FakeDelivery::scripted([Reply::Transport]);
    let alert = alert("https://hooks/a", AlertType::BudgetAlerts, "hello", None);
    assert!(!send_to_webhook(&delivery, false, &alert, 1).await);
    let contents = logs.contents();
    assert!(contents.contains("DEBUG"));
    assert!(contents.contains("the webhook alert could not be delivered"));
    assert!(contents.contains("the fake transport failed"));
}

#[tokio::test]
async fn an_unparsable_url_is_skipped_and_logged_at_debug() {
    let (logs, _guard) = capture_logs();
    let delivery = FakeDelivery::scripted([]);
    let alert = alert("not a url", AlertType::BudgetAlerts, "hello", None);
    assert!(!send_to_webhook(&delivery, false, &alert, 1).await);
    assert_eq!(delivery.request_count(), 0);
    let contents = logs.contents();
    assert!(contents.contains("DEBUG"));
    assert!(contents.contains("the webhook alert request could not be built"));
}

#[tokio::test]
async fn an_invalid_header_name_is_skipped_and_logged_at_debug() {
    let (logs, _guard) = capture_logs();
    let delivery = FakeDelivery::scripted([]);
    let alert = with_header(
        alert("https://hooks/a", AlertType::BudgetAlerts, "hello", None),
        "bad name",
        "value",
    );
    assert!(!send_to_webhook(&delivery, false, &alert, 1).await);
    assert_eq!(delivery.request_count(), 0);
    assert!(
        logs.contents()
            .contains("the webhook alert request could not be built")
    );
}

#[tokio::test]
async fn an_invalid_header_value_is_skipped_and_logged_at_debug() {
    let (logs, _guard) = capture_logs();
    let delivery = FakeDelivery::scripted([]);
    let alert = with_header(
        alert("https://hooks/a", AlertType::BudgetAlerts, "hello", None),
        "x-bad",
        "one\ntwo",
    );
    assert!(!send_to_webhook(&delivery, false, &alert, 1).await);
    assert_eq!(delivery.request_count(), 0);
    assert!(
        logs.contents()
            .contains("the webhook alert request could not be built")
    );
}

#[tokio::test]
async fn the_console_warning_prints_the_counted_message_even_when_delivery_fails() {
    let (logs, _guard) = capture_logs();
    let delivery = FakeDelivery::scripted([Reply::Transport]);
    let alert = alert("https://hooks/a", AlertType::BudgetAlerts, "hello", None);
    assert!(!send_to_webhook(&delivery, true, &alert, 2).await);
    let contents = logs.contents();
    assert!(contents.contains("WARN"));
    assert!(contents.contains("alert payload"));
    assert!(contents.contains("[Num Alerts: 2]"));
}

#[tokio::test]
async fn console_logging_stays_off_when_log_to_console_is_false() {
    let (logs, _guard) = capture_logs();
    let delivery = FakeDelivery::scripted([Reply::Status(500)]);
    let alert = alert("https://hooks/a", AlertType::BudgetAlerts, "hello", None);
    assert!(!send_to_webhook(&delivery, false, &alert, 1).await);
    assert!(!logs.contents().contains("alert payload"));
}

#[tokio::test]
async fn a_single_alert_sends_its_text_unchanged() {
    let delivery = FakeDelivery::scripted([Reply::Status(200)]);
    let alert = alert("https://hooks/a", AlertType::BudgetAlerts, "hello", None);
    assert!(send_to_webhook(&delivery, false, &alert, 1).await);
    let requests = delivery.take_requests();
    assert_eq!(requests[0].url, "https://hooks/a");
    assert_eq!(requests[0].body_as::<TextPayload>().text, "hello");
    assert_eq!(
        requests[0].headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(requests[0].deadline, Duration::from_secs(600));
}

#[tokio::test]
async fn repeated_alerts_prefix_the_text_with_the_number_of_alerts() {
    let delivery = FakeDelivery::scripted([Reply::Status(200)]);
    let alert = alert("https://hooks/a", AlertType::BudgetAlerts, "hello", None);
    assert!(send_to_webhook(&delivery, false, &alert, 3).await);
    let requests = delivery.take_requests();
    assert_eq!(
        requests[0].body_as::<TextPayload>().text,
        "[Num Alerts: 3]\n\nhello"
    );
}

#[tokio::test]
async fn a_teams_alert_sends_an_adaptive_card_with_the_counted_text() {
    let delivery = FakeDelivery::scripted([Reply::Status(200)]);
    let alert = alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "hello",
        Some("ms_teams"),
    );
    assert!(send_to_webhook(&delivery, false, &alert, 2).await);
    let requests = delivery.take_requests();
    let card = requests[0].body_as::<TeamsWireMessage>();
    assert_eq!(card.kind, "message");
    assert_eq!(card.attachments.len(), 1);
    assert_eq!(
        card.attachments[0].content_type,
        "application/vnd.microsoft.card.adaptive"
    );
    assert_eq!(card.attachments[0].content.kind, "AdaptiveCard");
    assert_eq!(card.attachments[0].content.version, "1.4");
    assert_eq!(card.attachments[0].content.body.len(), 1);
    assert_eq!(card.attachments[0].content.body[0].kind, "TextBlock");
    assert_eq!(
        card.attachments[0].content.body[0].text,
        "[Num Alerts: 2]\n\nhello"
    );
    assert!(card.attachments[0].content.body[0].wrap);
}

#[tokio::test]
async fn async_send_batch_squashes_identical_alerts_into_one_post() {
    let delivery = FakeDelivery::scripted([Reply::Status(200)]);
    let queue = queue();
    let _first = queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "hello",
        None,
    ));
    let _second = queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "hello",
        None,
    ));
    let batch = super::WebhookBatch {
        delivery: &delivery,
        log_to_console: false,
    };
    let report = queue.flush(&batch).await;
    assert_eq!(report.batch, 2);
    assert_eq!(report.delivered, 2);
    assert_eq!(delivery.request_count(), 1);
    let requests = delivery.take_requests();
    assert_eq!(
        requests[0].body_as::<TextPayload>().text,
        "[Num Alerts: 2]\n\nhello"
    );
}

#[tokio::test(start_paused = true)]
async fn async_send_batch_posts_distinct_alerts_concurrently() {
    let delivery = FakeDelivery::gated(3);
    let queue = queue();
    let _first = queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "one",
        None,
    ));
    let _second = queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "two",
        None,
    ));
    let _third = queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "three",
        None,
    ));
    let batch = super::WebhookBatch {
        delivery: &delivery,
        log_to_console: false,
    };
    timeout(Duration::from_secs(1), queue.flush(&batch))
        .await
        .expect("every squashed alert reaches the barrier at the same time");
    assert_eq!(delivery.request_count(), 3);
}

#[tokio::test]
async fn async_send_batch_clears_the_queue_after_sending() {
    let delivery = FakeDelivery::scripted([Reply::Status(200)]);
    let queue = queue();
    let _first = queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "one",
        None,
    ));
    let batch = super::WebhookBatch {
        delivery: &delivery,
        log_to_console: false,
    };
    let report = queue.flush(&batch).await;
    assert_eq!(report.requeued, 0);
    assert!(queue.pending().is_empty());
}

#[tokio::test]
async fn async_send_batch_clears_the_queue_when_delivery_fails() {
    let delivery = FakeDelivery::scripted([Reply::Transport]);
    let queue = queue();
    let _first = queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "one",
        None,
    ));
    let batch = super::WebhookBatch {
        delivery: &delivery,
        log_to_console: false,
    };
    let report = queue.flush(&batch).await;
    assert_eq!(report.delivered, 1);
    assert_eq!(report.requeued, 0);
    assert!(queue.pending().is_empty());
}

#[tokio::test]
async fn an_alert_enqueued_during_a_flush_stays_queued() {
    let (release, released) = oneshot::channel();
    let delivery = FakeDelivery::with_release(released);
    let queue = queue();
    let _first = queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "one",
        None,
    ));
    let batch = super::WebhookBatch {
        delivery: &delivery,
        log_to_console: false,
    };
    let mut flush = pin!(queue.flush(&batch));
    let pending_post = poll_fn(|context| Poll::Ready(Future::poll(flush.as_mut(), context))).await;
    assert!(pending_post.is_pending());
    let _second = queue.enqueue(alert(
        "https://hooks/a",
        AlertType::BudgetAlerts,
        "two",
        None,
    ));
    release.send(()).unwrap();
    let report = flush.await;
    assert_eq!(report.delivered, 1);
    let pending = queue.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].payload.text, "two");
}
