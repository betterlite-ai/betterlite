use std::sync::Arc;
use std::time::Duration;

use crate::gdc_auth::exchange::{Exchange, ExchangeError, ExchangeRequest};
use crate::gdc_auth::transport::{TokenExchangeResponse, TokenTrust};

use super::support::{FakeClock, FakeSleeper, RangeJitter, Reply, ScriptedTransport};

fn request() -> ExchangeRequest {
    ExchangeRequest {
        url: "https://identity.example/authenticate".to_owned(),
        body: "{\"subject_token\":\"signed\"}".to_owned(),
        trust: TokenTrust::System,
    }
}

fn answer(status: u16, body: &[u8]) -> Reply {
    Reply::Answer {
        status,
        body: body.to_vec(),
    }
}

struct Fixture {
    clock: Arc<FakeClock>,
    transport: Arc<ScriptedTransport>,
    sleeper: Arc<FakeSleeper>,
    jitter: Arc<RangeJitter>,
}

impl Fixture {
    fn new(replies: Vec<Reply>) -> Self {
        let clock = FakeClock::at(1_700_000_000);
        Self {
            sleeper: Arc::new(FakeSleeper::new(Arc::clone(&clock))),
            jitter: Arc::new(RangeJitter::default()),
            transport: Arc::new(ScriptedTransport::new(replies)),
            clock,
        }
    }

    async fn exchange(&self, deadline_millis: u64) -> Result<TokenExchangeResponse, ExchangeError> {
        let exchange = Exchange {
            transport: &*self.transport,
            sleeper: &*self.sleeper,
            jitter: &*self.jitter,
            clock: &*self.clock,
        };
        exchange.run(request(), deadline_millis).await
    }
}

async fn reply(fixture: &Fixture, deadline_millis: u64) -> TokenExchangeResponse {
    match fixture.exchange(deadline_millis).await {
        Ok(response) => response,
        Err(error) => panic!("expected a token reply, got {error}"),
    }
}

async fn failure(fixture: &Fixture, deadline_millis: u64) -> ExchangeError {
    match fixture.exchange(deadline_millis).await {
        Ok(_response) => panic!("expected an exchange failure"),
        Err(error) => error,
    }
}

#[tokio::test]
async fn retryable_statuses_use_three_attempts_with_one_then_two_second_jitter() {
    let fixture = Fixture::new(vec![
        answer(500, b""),
        answer(503, b""),
        answer(200, b"{\"access_token\":\"t\"}"),
    ]);
    let response = reply(&fixture, 60_000).await;
    assert_eq!(response.status, 200);
    assert_eq!(fixture.transport.count(), 3);
    assert_eq!(fixture.sleeper.waits(), vec![900, 1_800]);
    assert_eq!(fixture.jitter.ranges(), vec![(900, 1_100), (1_800, 2_200)]);
}

#[rstest::rstest]
#[case::internal("internal_failure")]
#[case::server("server_error")]
#[case::unavailable("temporarily_unavailable")]
#[tokio::test]
async fn each_retryable_error_description_is_retried(#[case] description: &str) {
    let body = format!("{{\"error\":\"{description}\"}}").into_bytes();
    let fixture = Fixture::new(vec![
        answer(400, &body),
        answer(200, b"{\"access_token\":\"t\"}"),
    ]);
    let response = reply(&fixture, 60_000).await;
    assert_eq!(response.status, 200);
    assert_eq!(fixture.transport.count(), 2);
}

#[tokio::test]
async fn a_retryable_error_description_in_the_error_description_field_is_retried() {
    let fixture = Fixture::new(vec![
        answer(400, b"{\"error_description\":\"server_error\"}"),
        answer(200, b"{}"),
    ]);
    let response = reply(&fixture, 60_000).await;
    assert_eq!(response.status, 200);
    assert_eq!(fixture.transport.count(), 2);
}

#[tokio::test]
async fn other_failures_stop_immediately() {
    let fixture = Fixture::new(vec![answer(400, b"{\"error\":\"invalid_grant\"}")]);
    let error = failure(&fixture, 60_000).await;
    assert!(matches!(error, ExchangeError::Rejected { status: 400 }));
    assert_eq!(fixture.transport.count(), 1);
    assert!(fixture.sleeper.waits().is_empty());
}

#[tokio::test]
async fn a_retryable_status_with_an_unreadable_body_still_retries() {
    let fixture = Fixture::new(vec![
        answer(500, b"oops"),
        answer(500, b"oops"),
        answer(500, b"oops"),
    ]);
    let error = failure(&fixture, 60_000).await;
    assert!(matches!(error, ExchangeError::Rejected { status: 500 }));
    assert_eq!(fixture.transport.count(), 3);
}

#[tokio::test]
async fn a_body_that_is_not_an_object_is_not_retryable() {
    let fixture = Fixture::new(vec![answer(400, b"[]")]);
    let error = failure(&fixture, 60_000).await;
    assert!(matches!(error, ExchangeError::Rejected { status: 400 }));
    assert_eq!(fixture.transport.count(), 1);
}

#[tokio::test]
async fn a_transport_failure_stops_without_another_attempt() {
    let fixture = Fixture::new(vec![Reply::Failure, answer(200, b"{}")]);
    let error = failure(&fixture, 60_000).await;
    assert!(matches!(error, ExchangeError::Transport { .. }));
    assert_eq!(fixture.transport.count(), 1);
}

#[tokio::test]
async fn only_status_200_is_a_successful_reply() {
    let fixture = Fixture::new(vec![answer(204, b"")]);
    let error = failure(&fixture, 60_000).await;
    assert!(matches!(error, ExchangeError::Rejected { status: 204 }));
    assert_eq!(fixture.transport.count(), 1);
}

#[tokio::test]
async fn the_deadline_bounds_the_wait_between_attempts() {
    let fixture = Fixture::new(vec![answer(500, b""), answer(500, b"")]);
    let error = failure(&fixture, 1_000).await;
    assert!(matches!(error, ExchangeError::Deadline));
    assert_eq!(fixture.transport.count(), 2);
    assert_eq!(fixture.sleeper.waits(), vec![900, 100]);
}

#[tokio::test]
async fn a_zero_deadline_never_sends_a_request() {
    let fixture = Fixture::new(vec![answer(200, b"{}")]);
    let error = failure(&fixture, 0).await;
    assert!(matches!(error, ExchangeError::Deadline));
    assert_eq!(fixture.transport.count(), 0);
}

#[tokio::test]
async fn each_attempt_carries_the_remaining_deadline_and_the_same_body() {
    let fixture = Fixture::new(vec![answer(500, b""), answer(200, b"{}")]);
    let response = reply(&fixture, 60_000).await;
    assert_eq!(response.status, 200);
    let records = fixture.transport.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].timeout, Duration::from_millis(60_000));
    assert_eq!(records[1].timeout, Duration::from_millis(59_100));
    assert_eq!(records[0].body, "{\"subject_token\":\"signed\"}");
    assert_eq!(records[1].body, records[0].body);
    assert_eq!(records[0].url, "https://identity.example/authenticate");
}
