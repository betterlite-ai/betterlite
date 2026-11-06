use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::xai_login::error::XaiLoginError;
use crate::xai_login::listener::{
    CALLBACK_TIMEOUT, CallbackServer, CallbackSettings, MAX_REQUEST_BYTES, POLL_INTERVAL,
    PREFERRED_PORT, wait_slot,
};

use super::support::{
    FixedClock, NOW, STATE, SteppingClock, capture_events, get, send, server_port, transcript,
};

fn settings() -> CallbackSettings {
    CallbackSettings {
        preferred_port: 0,
        poll: Duration::from_millis(5),
        timeout: Duration::from_secs(30),
    }
}

#[test]
fn the_callback_defaults_match_the_pinned_port_and_deadline() {
    let settings = CallbackSettings::default();
    assert_eq!(settings.preferred_port, 56_121);
    assert_eq!(settings.poll, Duration::from_secs(1));
    assert_eq!(settings.timeout, Duration::from_secs(180));
    assert_eq!(PREFERRED_PORT, 56_121);
    assert_eq!(POLL_INTERVAL, Duration::from_secs(1));
    assert_eq!(CALLBACK_TIMEOUT, Duration::from_secs(180));
}

#[rstest]
#[case::after_the_deadline("1001", None)]
#[case::at_the_deadline("1000", None)]
#[case::one_second_left("999", Some(Duration::from_secs(1)))]
#[case::half_a_second_left("999.5", Some(Duration::from_millis(500)))]
#[case::capped_by_the_poll("990", Some(Duration::from_secs(1)))]
fn computes_the_poll_slot(#[case] now: &str, #[case] expected: Option<Duration>) {
    let slot = wait_slot(
        now.parse().unwrap(),
        Decimal::from(1_000_i32),
        POLL_INTERVAL,
    );
    assert_eq!(slot, expected);
}

#[rstest]
#[case::matching_state_and_code("matching_state_and_code")]
#[case::matching_state_with_provider_error("matching_state_with_provider_error")]
#[case::matching_state_without_code("matching_state_without_code")]
#[case::state_mismatch("state_mismatch")]
#[case::duplicate_query_fields_keep_the_first("duplicate_query_fields_keep_the_first")]
#[case::percent_escaped_values("percent_escaped_values")]
#[case::blank_query_fields_are_dropped("blank_query_fields_are_dropped")]
#[case::unrelated_path("unrelated_path")]
#[tokio::test]
async fn reproduces_the_pinned_python_callback_transcript(#[case] name: &str) {
    let transcript = transcript();
    let case = transcript
        .cases
        .iter()
        .find(|case| case.name == name)
        .unwrap();
    let clock = FixedClock(Decimal::from(NOW));
    let server = CallbackServer::bind(&settings(), &transcript.expected_state)
        .await
        .unwrap();
    let port = server_port(&server);
    let (response, outcome) = tokio::join!(
        get(port, &case.target),
        timeout(Duration::from_millis(1_000), server.wait(&clock)),
    );
    assert_eq!(response.status, case.status);
    assert_eq!(response.content_type, case.content_type);
    assert_eq!(response.body, case.body);
    let Some(expected) = &case.callback else {
        outcome.unwrap_err();
        return;
    };
    let fields = outcome.unwrap().unwrap();
    assert_eq!(
        expected.code.as_deref(),
        fields.code.as_ref().map(ExposeSecret::expose_secret),
    );
    assert_eq!(expected.state, fields.state);
    assert_eq!(expected.error, fields.error);
    assert_eq!(expected.error_description, fields.error_description);
}

#[test]
fn the_pinned_transcript_lists_each_case_once() {
    let transcript = transcript();
    let names: Vec<&str> = transcript
        .cases
        .iter()
        .map(|case| case.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "matching_state_and_code",
            "matching_state_with_provider_error",
            "matching_state_without_code",
            "state_mismatch",
            "duplicate_query_fields_keep_the_first",
            "percent_escaped_values",
            "blank_query_fields_are_dropped",
            "unrelated_path",
        ]
    );
}

#[tokio::test]
async fn an_unrelated_path_keeps_the_wait_running() {
    let clock = FixedClock(Decimal::from(NOW));
    let server = CallbackServer::bind(&settings(), STATE).await.unwrap();
    let port = server_port(&server);
    let wait = server.wait(&clock);
    tokio::pin!(wait);
    let (response, outcome) = tokio::join!(
        get(port, "/favicon.ico"),
        timeout(Duration::from_millis(150), &mut wait),
    );
    assert_eq!(response.status, 404);
    assert_eq!(response.content_type, None);
    outcome.unwrap_err();
    let (received, waiting) = tokio::join!(
        get(port, "/callback?code=code-1&state=state-test"),
        &mut wait,
    );
    assert_eq!(received.status, 200);
    let fields = waiting.unwrap();
    assert_eq!(
        fields.code.as_ref().map(ExposeSecret::expose_secret),
        Some("code-1")
    );
    assert_eq!(fields.state.as_deref(), Some(STATE));
}

#[tokio::test]
async fn a_method_other_than_get_keeps_the_wait_running() {
    let clock = FixedClock(Decimal::from(NOW));
    let server = CallbackServer::bind(&settings(), STATE).await.unwrap();
    let port = server_port(&server);
    let wait = server.wait(&clock);
    tokio::pin!(wait);
    let (response, outcome) = tokio::join!(
        send(port, "POST /callback HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
        timeout(Duration::from_millis(150), &mut wait),
    );
    assert_eq!(response.status, 501);
    outcome.unwrap_err();
}

#[tokio::test]
async fn an_oversized_request_head_is_refused_and_keeps_waiting() {
    let clock = FixedClock(Decimal::from(NOW));
    let server = CallbackServer::bind(&settings(), STATE).await.unwrap();
    let port = server_port(&server);
    let wait = server.wait(&clock);
    tokio::pin!(wait);
    let request = format!(
        "GET /callback?code={} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        "a".repeat(MAX_REQUEST_BYTES)
    );
    let (response, outcome) = tokio::join!(
        send(port, &request),
        timeout(Duration::from_millis(300), &mut wait),
    );
    assert_eq!(response.status, 431);
    outcome.unwrap_err();
}

#[tokio::test]
async fn the_receiver_writes_no_access_log() {
    let (captured, _guard) = capture_events();
    let clock = FixedClock(Decimal::from(NOW));
    let server = CallbackServer::bind(&settings(), STATE).await.unwrap();
    let port = server_port(&server);
    let wait = server.wait(&clock);
    tokio::pin!(wait);
    let (missing, outcome) = tokio::join!(
        get(port, "/favicon.ico"),
        timeout(Duration::from_millis(150), &mut wait),
    );
    assert_eq!(missing.status, 404);
    outcome.unwrap_err();
    let (received, fields) = tokio::join!(
        get(port, "/callback?code=code-1&state=state-test"),
        &mut wait,
    );
    assert_eq!(received.status, 200);
    fields.unwrap();
    assert!(captured.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_taken_preferred_port_falls_back_to_an_ephemeral_port() {
    let occupied = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    let taken = occupied.local_addr().unwrap().port();
    let settings = CallbackSettings {
        preferred_port: taken,
        poll: Duration::from_millis(5),
        timeout: Duration::from_secs(30),
    };
    let clock = FixedClock(Decimal::from(NOW));
    let server = CallbackServer::bind(&settings, STATE).await.unwrap();
    assert_ne!(server_port(&server), taken);
    assert_eq!(
        server.redirect_uri,
        format!("http://127.0.0.1:{}/callback", server_port(&server))
    );
    let port = server_port(&server);
    let (response, fields) = tokio::join!(
        get(port, "/callback?code=code-1&state=state-test"),
        server.wait(&clock),
    );
    assert_eq!(response.status, 200);
    assert_eq!(
        fields
            .unwrap()
            .code
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("code-1")
    );
}

#[tokio::test]
async fn the_wait_times_out_at_the_deadline() {
    let clock = SteppingClock {
        now: Mutex::new(Decimal::from(NOW)),
        step: Decimal::from(1_i32),
    };
    let settings = CallbackSettings {
        preferred_port: 0,
        poll: Duration::from_millis(5),
        timeout: Duration::from_secs(3),
    };
    let server = CallbackServer::bind(&settings, STATE).await.unwrap();
    assert!(matches!(
        server.wait(&clock).await,
        Err(XaiLoginError::Timeout)
    ));
}

#[rstest]
#[case::after_a_callback(true)]
#[case::after_a_timeout(false)]
#[tokio::test]
async fn the_listener_is_closed_when_the_wait_ends(#[case] deliver: bool) {
    let clock = FixedClock(Decimal::from(NOW));
    let server = CallbackServer::bind(&settings(), STATE).await.unwrap();
    let port = server_port(&server);
    if deliver {
        let (response, _fields) = tokio::join!(
            get(port, "/callback?code=code-1&state=state-test"),
            server.wait(&clock),
        );
        assert_eq!(response.status, 200);
    } else {
        let stepping = SteppingClock {
            now: Mutex::new(Decimal::from(NOW)),
            step: Decimal::from(30_i32),
        };
        assert!(matches!(
            server.wait(&stepping).await,
            Err(XaiLoginError::Timeout)
        ));
    }
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .await
        .unwrap();
}
