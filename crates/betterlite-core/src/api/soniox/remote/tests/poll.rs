use std::time::Duration;

use http::Method;
use http::header::AUTHORIZATION;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

use super::support::{
    Clock, Outcome, RecordedBody, Script, TIMEOUT, auth, options, prepared, remote,
};
use crate::api::soniox::prepare::options::HandlerOptions;
use crate::api::soniox::prepare::response::TranscriptionMeta;
use crate::api::soniox::remote::error::RemoteError;
use crate::api::soniox::remote::poll::poll_until_completed;

async fn poll_with(
    answers: Vec<Outcome>,
    chosen: HandlerOptions,
    id: &str,
) -> (Result<TranscriptionMeta, RemoteError>, Script, Clock) {
    let transport = Script::new(answers);
    let sleeper = Clock::default();
    let ready = prepared(chosen, &auth());
    let outcome = poll_until_completed(&remote(&transport, &sleeper, &ready), id).await;
    (outcome, transport, sleeper)
}

fn completed() -> Outcome {
    Outcome::Reply {
        status: 200,
        headers: vec![("x-request-id", "poll-1")],
        body: r#"{"status":"completed","audio_duration_ms":1500}"#,
    }
}

#[tokio::test]
async fn poll_returns_the_completed_metadata() {
    let (outcome, transport, sleeper) = poll_with(vec![completed()], options(), "job-1").await;
    let meta = outcome.unwrap();
    assert_eq!(meta.status.as_deref(), Some("completed"));
    assert_eq!(meta.audio_duration_ms.unwrap().to_string(), "1500");
    assert_eq!(
        meta.raw.get("audio_duration_ms").unwrap().to_string(),
        "1500"
    );
    assert!(sleeper.sleeps().is_empty());
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, Method::GET);
    assert_eq!(
        calls[0].url,
        "https://api.soniox.test/v1/transcriptions/job-1"
    );
    assert_eq!(calls[0].headers.len(), 2);
    assert_eq!(
        calls[0]
            .headers
            .get(AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer sk-local"
    );
    assert!(matches!(&calls[0].body, RecordedBody::Empty));
    assert_eq!(calls[0].deadline, TIMEOUT);
}

#[tokio::test]
async fn poll_keeps_waiting_on_an_upper_case_status() {
    let mut chosen = options();
    chosen.max_attempts = 2;
    let upper = || Outcome::Reply {
        status: 200,
        headers: Vec::new(),
        body: r#"{"status":"COMPLETED"}"#,
    };
    let (outcome, transport, sleeper) = poll_with(vec![upper(), upper()], chosen, "job-1").await;
    assert_timeout(outcome, "2", "1.0");
    assert_eq!(transport.calls().len(), 2);
    assert_eq!(sleeper.sleeps(), vec![Duration::from_millis(1000); 2]);
}

#[tokio::test]
async fn poll_waits_once_after_the_only_attempt() {
    let mut chosen = options();
    chosen.max_attempts = 1;
    let body = Outcome::Reply {
        status: 200,
        headers: Vec::new(),
        body: "{}",
    };
    let (outcome, transport, sleeper) = poll_with(vec![body], chosen, "job-1").await;
    assert_timeout(outcome, "1", "1.0");
    assert_eq!(transport.calls().len(), 1);
    assert_eq!(sleeper.sleeps(), vec![Duration::from_millis(1000)]);
}

#[tokio::test]
async fn poll_sleeps_the_prepared_interval() {
    let mut chosen = options();
    chosen.max_attempts = 1;
    chosen.poll_interval = Decimal::new(15, 1);
    let body = Outcome::Reply {
        status: 200,
        headers: Vec::new(),
        body: r#"{"status":"queued"}"#,
    };
    let (outcome, transport, sleeper) = poll_with(vec![body], chosen, "job-1").await;
    assert_timeout(outcome, "1", "1.5");
    assert_eq!(transport.calls().len(), 1);
    assert_eq!(sleeper.sleeps(), vec![Duration::from_millis(1500)]);
}

fn assert_timeout(outcome: Result<TranscriptionMeta, RemoteError>, attempts: &str, interval: &str) {
    let RemoteError::Provider(provider) = outcome.unwrap_err() else {
        panic!("an unfinished transcription must end in the timeout");
    };
    assert_eq!(provider.status_code, 504);
    assert_eq!(
        provider.message,
        format!(
            "Soniox transcription job-1 did not complete after {attempts} polling attempts (interval={interval}s)."
        )
    );
    assert!(provider.headers.unwrap().is_empty());
}

#[tokio::test]
async fn poll_raises_the_terminal_error() {
    let body = Outcome::Reply {
        status: 200,
        headers: vec![("x-request-id", "poll-2")],
        body: r#"{"status":"error","error_message":"bad audio"}"#,
    };
    let (outcome, transport, sleeper) = poll_with(vec![body], options(), "job-1").await;
    let RemoteError::Provider(provider) = outcome.unwrap_err() else {
        panic!("a terminal error must surface as the provider error");
    };
    assert_eq!(provider.status_code, 500);
    assert_eq!(
        provider.message,
        "Soniox transcription job-1 failed: bad audio"
    );
    let headers = provider.headers.unwrap();
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("poll-2")
    );
    assert_eq!(transport.calls().len(), 1);
    assert!(sleeper.sleeps().is_empty());
}

#[tokio::test]
async fn poll_falls_back_to_the_error_type() {
    let body = Outcome::Reply {
        status: 200,
        headers: Vec::new(),
        body: r#"{"status":"error","error_message":null,"error_type":"invalid_audio"}"#,
    };
    let (outcome, _, _) = poll_with(vec![body], options(), "job-1").await;
    let RemoteError::Provider(provider) = outcome.unwrap_err() else {
        panic!("a terminal error must surface as the provider error");
    };
    assert_eq!(
        provider.message,
        "Soniox transcription job-1 failed: invalid_audio"
    );
}

#[tokio::test]
async fn poll_reports_unknown_when_the_error_has_no_detail() {
    let body = Outcome::Reply {
        status: 200,
        headers: Vec::new(),
        body: r#"{"status":"error","error_message":"","error_type":null}"#,
    };
    let (outcome, _, _) = poll_with(vec![body], options(), "job-1").await;
    let RemoteError::Provider(provider) = outcome.unwrap_err() else {
        panic!("a terminal error must surface as the provider error");
    };
    assert_eq!(
        provider.message,
        "Soniox transcription job-1 failed: unknown error"
    );
}

#[tokio::test]
async fn poll_propagates_an_http_failure_without_retrying() {
    let failure = Outcome::Reply {
        status: 500,
        headers: Vec::new(),
        body: r#"{"error_message":"upstream died"}"#,
    };
    let (outcome, transport, sleeper) =
        poll_with(vec![failure, completed()], options(), "job-1").await;
    let RemoteError::Provider(provider) = outcome.unwrap_err() else {
        panic!("an HTTP failure must surface as the provider error");
    };
    assert_eq!(provider.status_code, 500);
    assert_eq!(
        provider.message,
        "Soniox poll transcription failed (HTTP 500): upstream died"
    );
    assert_eq!(transport.calls().len(), 1);
    assert!(sleeper.sleeps().is_empty());
}

#[tokio::test]
async fn poll_prints_the_prepared_interval_at_decimal_precision() {
    let mut chosen = options();
    chosen.max_attempts = 1;
    chosen.poll_interval = Decimal::from_f64(2.500_000_000_000_000_4).expect("interval");
    let body = Outcome::Reply {
        status: 200,
        headers: Vec::new(),
        body: r#"{"status":"queued"}"#,
    };
    let (outcome, _, sleeper) = poll_with(vec![body], chosen, "job-1").await;
    assert_timeout(outcome, "1", "2.5");
    assert_eq!(sleeper.sleeps(), vec![Duration::from_millis(2500)]);
}

#[tokio::test]
async fn poll_reports_a_header_the_request_cannot_carry() {
    let transport = Script::new(vec![completed()]);
    let sleeper = Clock::default();
    let ready = prepared(
        options(),
        &[
            ("Authorization", "Bearer sk-local"),
            ("x-bad", "line\nbreak"),
        ],
    );
    let error = poll_until_completed(&remote(&transport, &sleeper, &ready), "job-1")
        .await
        .unwrap_err();
    let RemoteError::HeaderValue { name, .. } = error else {
        panic!("a header a request cannot carry must fail before the request is sent");
    };
    assert_eq!(name, "x-bad");
    assert!(transport.calls().is_empty());
}

#[tokio::test]
async fn poll_joins_repeated_reply_headers_and_reads_text() {
    let body = Outcome::Reply {
        status: 200,
        headers: vec![("via", "1.1 a"), ("via", "1.1 b"), ("x-note", "caf\u{e9}")],
        body: r#"{"status":"error","error_message":"bad audio"}"#,
    };
    let (outcome, _, _) = poll_with(vec![body], options(), "job-1").await;
    let RemoteError::Provider(provider) = outcome.unwrap_err() else {
        panic!("a terminal error must surface as the provider error");
    };
    let headers = provider.headers.unwrap();
    assert_eq!(headers.get("via").map(String::as_str), Some("1.1 a, 1.1 b"));
    assert_eq!(headers.get("x-note").map(String::as_str), Some("caf\u{e9}"));
}

#[tokio::test]
async fn poll_interpolates_the_identifier_literally() {
    let (outcome, transport, _) = poll_with(vec![completed()], options(), "job/42").await;
    outcome.unwrap();
    assert_eq!(
        transport.calls()[0].url,
        "https://api.soniox.test/v1/transcriptions/job/42"
    );
}
