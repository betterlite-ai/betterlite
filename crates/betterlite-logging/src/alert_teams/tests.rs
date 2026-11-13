#![cfg(test)]

use std::collections::BTreeMap;
use std::io::{Result as IoResult, Write};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::alert_settings::types::AlertType;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use super::{
    MS_TEAMS_ALERTING_DESTINATION, MS_TEAMS_WEBHOOK_URL_ENV, build_ms_teams_payload,
    enqueue_ms_teams_alert,
};
use crate::alert_settings::records::AlertQueueItem;
use crate::batch::logger::{BatchQueue, BatchSettings};

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn contents(&self) -> String {
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

fn capture_logs() -> (LogBuffer, DefaultGuard) {
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

fn lookup(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn queue() -> BatchQueue<AlertQueueItem> {
    BatchQueue::new(BatchSettings::default())
}

#[test]
fn the_teams_payload_is_an_adaptive_card_version_1_4_with_one_wrapped_text_block() {
    let payload = build_ms_teams_payload("the alert");
    assert_eq!(payload.kind, "message");
    assert_eq!(payload.attachments.len(), 1);
    let attachment = &payload.attachments[0];
    assert_eq!(
        attachment.content_type,
        "application/vnd.microsoft.card.adaptive"
    );
    assert_eq!(attachment.content.kind, "AdaptiveCard");
    assert_eq!(attachment.content.version, "1.4");
    assert_eq!(attachment.content.body.len(), 1);
    let block = &attachment.content.body[0];
    assert_eq!(block.kind, "TextBlock");
    assert_eq!(block.text, "the alert");
    assert!(block.wrap);
}

#[rstest]
#[case::set(&[(MS_TEAMS_WEBHOOK_URL_ENV, "https://teams.example/webhook")], true)]
#[case::unset(&[], false)]
#[case::empty(&[(MS_TEAMS_WEBHOOK_URL_ENV, "")], false)]
fn the_teams_webhook_url_is_resolved_from_the_environment(
    #[case] variables: &[(&str, &str)],
    #[case] enqueued: bool,
) {
    let (logs, _guard) = capture_logs();
    let environment = lookup(variables);
    let queue = queue();
    let result = enqueue_ms_teams_alert(&queue, &environment, "the alert", AlertType::BudgetAlerts);
    assert_eq!(result.is_some(), enqueued);
    assert_eq!(queue.pending().len(), usize::from(enqueued));
    if !enqueued {
        let contents = logs.contents();
        assert!(contents.contains("ERROR"));
        assert!(contents.contains(MS_TEAMS_WEBHOOK_URL_ENV));
        assert!(contents.contains("dropping the alert"));
    }
}

#[test]
fn the_teams_item_carries_the_alert_format_and_json_headers() {
    let environment = lookup(&[(MS_TEAMS_WEBHOOK_URL_ENV, "https://teams.example/webhook")]);
    let queue = queue();
    let enqueued = enqueue_ms_teams_alert(
        &queue,
        &environment,
        "Alert type: `budget_alerts`\nLevel: `High`",
        AlertType::BudgetAlerts,
    )
    .expect("the teams webhook URL is set");
    assert_eq!(enqueued.depth, 1);
    let items = queue.pending();
    assert_eq!(
        items[0].url.expose_secret(),
        "https://teams.example/webhook"
    );
    assert_eq!(
        items[0].headers.get("Content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        items[0].payload.text,
        "Alert type: `budget_alerts`\nLevel: `High`"
    );
    assert_eq!(items[0].alert_type, AlertType::BudgetAlerts);
    assert_eq!(
        items[0].format.as_deref(),
        Some(MS_TEAMS_ALERTING_DESTINATION)
    );
}
