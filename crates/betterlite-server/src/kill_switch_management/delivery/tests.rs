#![cfg(test)]

use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_core::agents::{AgentKillSwitchMethod, AgentKillSwitchResult};
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::{DEFAULT_TIMEOUT, RESPONSE_BODY_MAX_CHARS, fire_kill_switch};
use crate::kill_switch_management::client::KillSwitchSendOptions;
use crate::kill_switch_management::support::{
    FakeClient, config_with_auth, first_sent, lock, probe,
};

async fn fire_agent(client: &FakeClient) -> AgentKillSwitchResult {
    fire_kill_switch("agent-1", &config_with_auth(None), client, DEFAULT_TIMEOUT).await
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Logged {
    level: Level,
    message: String,
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<Logged>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        lock(&self.0).push(Logged {
            level: *event.metadata().level(),
            message: message.0,
        });
    }
}

fn capture() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let guard = set_default(registry().with(logs.clone()));
    (logs, guard)
}

fn messages(logs: &Logs) -> Vec<Logged> {
    lock(&logs.0).clone()
}

const FIRING: &str = "Firing kill switch for agent agent-1: POST https://agent.example/stop";

#[tokio::test]
async fn delivering_logs_the_request_it_fires() {
    let (logs, _guard) = capture();
    let client = FakeClient::scripted(200, Vec::new());
    fire_agent(&client).await;
    assert_eq!(
        messages(&logs),
        vec![Logged {
            level: Level::INFO,
            message: String::from(FIRING),
        }]
    );
}

#[tokio::test]
async fn delivering_logs_a_failed_attempt() {
    let (logs, _guard) = capture();
    let mut client = FakeClient::scripted(200, Vec::new());
    client.send_error = Some(String::from("connection refused"));
    fire_agent(&client).await;
    assert_eq!(
        messages(&logs),
        vec![
            Logged {
                level: Level::INFO,
                message: String::from(FIRING),
            },
            Logged {
                level: Level::WARN,
                message: String::from("Kill switch for agent agent-1 failed: connection refused"),
            },
        ]
    );
}

#[tokio::test]
async fn delivering_a_2xx_response_records_the_status_and_body() {
    let client = FakeClient::scripted(200, vec![Ok(String::from("stopped"))]);
    let result = fire_agent(&client).await;
    assert_eq!(result.status_code, Some(200));
    assert_eq!(result.response_body.as_deref(), Some("stopped"));
    assert!(result.error.is_none());
    assert!(result.succeeded());
}

#[rstest]
#[case::informational(199, false)]
#[case::ok(200, true)]
#[case::created(201, true)]
#[case::last_2xx(299, true)]
#[case::redirect(300, false)]
#[case::server_error(500, false)]
#[tokio::test]
async fn delivering_marks_only_2xx_as_succeeded(#[case] status: u16, #[case] succeeded: bool) {
    let client = FakeClient::scripted(status, Vec::new());
    let result = fire_agent(&client).await;
    assert_eq!(result.status_code, Some(status));
    assert_eq!(result.succeeded(), succeeded);
}

#[tokio::test]
async fn delivering_records_the_body_of_a_failed_status() {
    let client = FakeClient::scripted(500, vec![Ok(String::from("boom"))]);
    let result = fire_agent(&client).await;
    assert_eq!(result.status_code, Some(500));
    assert_eq!(result.response_body.as_deref(), Some("boom"));
    assert!(!result.succeeded());
}

#[tokio::test]
async fn delivering_sends_without_redirects_and_with_streaming() {
    let client = FakeClient::scripted(200, Vec::new());
    fire_agent(&client).await;
    let sent = first_sent(&client).expect("the request is sent");
    assert_eq!(
        sent.options,
        KillSwitchSendOptions {
            stream: true,
            follow_redirects: false,
        }
    );
    assert_eq!(sent.request.url, "https://agent.example/stop");
    assert_eq!(sent.request.method, AgentKillSwitchMethod::Post);
}

#[tokio::test]
async fn delivering_uses_the_default_timeout_of_ten_seconds() {
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(10));
    let client = FakeClient::scripted(200, Vec::new());
    fire_agent(&client).await;
    let sent = first_sent(&client).expect("the request is sent");
    assert_eq!(sent.timeout, Duration::from_secs(10));
}

#[tokio::test]
async fn delivering_uses_the_timeout_it_is_given() {
    let client = FakeClient::scripted(200, Vec::new());
    let config = config_with_auth(None);
    let result = fire_kill_switch("agent-1", &config, &client, Duration::from_secs(3)).await;
    assert!(result.succeeded());
    let sent = first_sent(&client).expect("the request is sent");
    assert_eq!(sent.timeout, Duration::from_secs(3));
}

#[tokio::test]
async fn delivering_reports_the_agent_url_and_method() {
    let mut config = config_with_auth(None);
    config.url = String::from("https://agent.example/stop?region=us");
    config.method = AgentKillSwitchMethod::Put;
    let client = FakeClient::scripted(204, Vec::new());
    let result = fire_kill_switch("agent-7", &config, &client, DEFAULT_TIMEOUT).await;
    assert_eq!(result.agent_id, "agent-7");
    assert_eq!(result.url, "https://agent.example/stop");
    assert_eq!(result.method, AgentKillSwitchMethod::Put);
}

#[tokio::test]
async fn delivering_cuts_the_body_at_two_thousand_characters() {
    assert_eq!(RESPONSE_BODY_MAX_CHARS, 2000);
    let client = FakeClient::scripted(200, vec![Ok("a".repeat(2500))]);
    let result = fire_agent(&client).await;
    assert_eq!(
        result.response_body.as_deref().map(str::len),
        Some(RESPONSE_BODY_MAX_CHARS)
    );
}

#[tokio::test]
async fn delivering_stops_reading_once_the_cap_is_reached() {
    let chunk = "a".repeat(RESPONSE_BODY_MAX_CHARS);
    let client = FakeClient::scripted(200, vec![Ok(chunk), Ok(String::from("extra"))]);
    let result = fire_agent(&client).await;
    assert_eq!(
        result.response_body.as_deref().map(str::len),
        Some(RESPONSE_BODY_MAX_CHARS)
    );
    assert_eq!(probe(&client).chunks_read, 1);
}

#[tokio::test]
async fn delivering_counts_the_cap_in_characters_not_bytes() {
    let client = FakeClient::scripted(200, vec![Ok("\u{e9}".repeat(2100))]);
    let result = fire_agent(&client).await;
    let body = result.response_body.expect("a body is read");
    assert_eq!(body.chars().count(), RESPONSE_BODY_MAX_CHARS);
    assert_eq!(body.len(), 4000);
}

#[tokio::test]
async fn delivering_records_an_empty_body_as_an_empty_string() {
    let client = FakeClient::scripted(204, Vec::new());
    let result = fire_agent(&client).await;
    assert_eq!(result.response_body.as_deref(), Some(""));
}

#[tokio::test]
async fn delivering_stops_at_an_empty_chunk() {
    let client = FakeClient::scripted(200, vec![Ok(String::new()), Ok(String::from("later"))]);
    let result = fire_agent(&client).await;
    assert_eq!(result.response_body.as_deref(), Some(""));
    assert_eq!(probe(&client).chunks_read, 1);
}

#[tokio::test]
async fn delivering_releases_the_response() {
    let client = FakeClient::scripted(200, vec![Ok(String::from("stopped"))]);
    fire_agent(&client).await;
    assert!(probe(&client).closed);
}

#[tokio::test]
async fn delivering_records_a_build_error() {
    let mut client = FakeClient::scripted(200, Vec::new());
    client.build_error = Some(String::from("header value is not ASCII"));
    let result = fire_agent(&client).await;
    assert_eq!(result.error.as_deref(), Some("header value is not ASCII"));
    assert!(result.status_code.is_none());
    assert!(first_sent(&client).is_none());
}

#[tokio::test]
async fn delivering_records_a_send_error() {
    let mut client = FakeClient::scripted(200, Vec::new());
    client.send_error = Some(String::from("connection refused"));
    let result = fire_agent(&client).await;
    assert_eq!(result.error.as_deref(), Some("connection refused"));
    assert!(result.status_code.is_none());
    assert!(result.response_body.is_none());
    assert!(!result.succeeded());
}

#[tokio::test]
async fn delivering_records_a_read_error_and_still_releases_the_response() {
    let client = FakeClient::scripted(
        200,
        vec![
            Ok(String::from("part")),
            Err(String::from("connection reset")),
        ],
    );
    let result = fire_agent(&client).await;
    assert_eq!(result.error.as_deref(), Some("connection reset"));
    assert!(result.status_code.is_none());
    assert!(result.response_body.is_none());
    let observed = probe(&client);
    assert_eq!(observed.chunks_read, 2);
    assert!(observed.closed);
}

#[tokio::test]
async fn delivering_records_a_close_error() {
    let mut client = FakeClient::scripted(200, vec![Ok(String::from("stopped"))]);
    client.close_error = Some(String::from("pool is gone"));
    let result = fire_agent(&client).await;
    assert_eq!(result.error.as_deref(), Some("pool is gone"));
    assert!(result.status_code.is_none());
    assert!(result.response_body.is_none());
    assert!(probe(&client).closed);
}

#[tokio::test]
async fn delivering_reports_a_close_error_over_a_read_error() {
    let mut client = FakeClient::scripted(200, vec![Err(String::from("read failed"))]);
    client.close_error = Some(String::from("close failed"));
    let result = fire_agent(&client).await;
    assert_eq!(result.error.as_deref(), Some("close failed"));
}

#[tokio::test]
async fn delivering_ignores_a_read_error_after_the_cap() {
    let chunk = "a".repeat(RESPONSE_BODY_MAX_CHARS);
    let client = FakeClient::scripted(200, vec![Ok(chunk), Err(String::from("late"))]);
    let result = fire_agent(&client).await;
    assert!(result.error.is_none());
    assert_eq!(
        result.response_body.as_deref().map(str::len),
        Some(RESPONSE_BODY_MAX_CHARS)
    );
    assert_eq!(probe(&client).chunks_read, 1);
}

#[tokio::test]
async fn delivering_records_an_invalid_url_as_an_error() {
    let mut config = config_with_auth(None);
    config.url = String::from("https://agent.example/stop\n");
    let client = FakeClient::scripted(200, Vec::new());
    let result = fire_kill_switch("agent-1", &config, &client, DEFAULT_TIMEOUT).await;
    let Some(error) = result.error.as_deref() else {
        panic!("the URL failure is recorded");
    };
    assert!(error.starts_with("the kill switch URL is not valid:"));
    assert!(result.status_code.is_none());
    assert!(first_sent(&client).is_none());
}
