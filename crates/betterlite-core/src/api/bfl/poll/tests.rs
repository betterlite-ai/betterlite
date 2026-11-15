#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, pending, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::HeaderMap;
use rstest::rstest;
use tokio::time::Instant;
use url::Url;

use super::{BflPoller, MonotonicClock, PollClock, PollError, PollSleeper, PollTransport};

const SUBMITTED: &str =
    r#"{"id":"job-1","polling_url":"https://gateway.bfl.ai/v1/get_result?id=1"}"#;
const PENDING: &str = r#"{"status":"Pending"}"#;
const READY: &str = r#"{"status":"Ready","result":{"sample":"https://delivery.test/a.png"}}"#;

#[derive(Clone)]
struct Reply {
    status: u16,
    body: &'static str,
}

struct Script {
    replies: Mutex<VecDeque<Reply>>,
    fallback: Reply,
    requests: Mutex<Vec<(String, HeaderMap)>>,
}

impl Script {
    fn once(status: u16, body: &'static str) -> Self {
        Self::new(
            vec![Reply { status, body }],
            Reply {
                status,
                body: PENDING,
            },
        )
    }

    fn new(replies: Vec<Reply>, fallback: Reply) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            fallback,
            requests: Mutex::default(),
        }
    }

    fn urls(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(url, _)| url.clone())
            .collect()
    }

    fn header_names(&self, index: usize) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(index)
            .map(|(_, headers)| {
                let mut names: Vec<String> = headers
                    .keys()
                    .map(|name| name.as_str().to_owned())
                    .collect();
                names.sort();
                names
            })
            .unwrap_or_default()
    }
}

impl PollTransport for Script {
    async fn get(&self, url: Url, headers: HeaderMap) -> Result<UpstreamResponse, TransportError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url.to_string(), headers));
        let reply = {
            let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
            replies.pop_front().unwrap_or_else(|| self.fallback.clone())
        };
        Ok(UpstreamResponse {
            status: reply.status,
            headers: HeaderMap::new(),
            body: reply.body.as_bytes().to_vec(),
        })
    }
}

struct StepClock {
    millis: AtomicU64,
    step: u64,
}

impl PollClock for StepClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.millis.fetch_add(self.step, Ordering::SeqCst))
    }
}

struct Recorder<'clock> {
    clock: &'clock StepClock,
    slept: Mutex<Vec<Duration>>,
}

impl PollSleeper for Recorder<'_> {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.slept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
        let step = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        self.clock.millis.fetch_add(step, Ordering::SeqCst);
        ready(())
    }
}

struct Token(Arc<AtomicBool>);

impl Drop for Token {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct HangingGet {
    cancelled: Arc<AtomicBool>,
}

impl PollTransport for HangingGet {
    fn get(
        &self,
        _url: Url,
        _headers: HeaderMap,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let token = Token(Arc::clone(&self.cancelled));
        async move {
            let _keep = token;
            pending::<Result<UpstreamResponse, TransportError>>().await
        }
    }
}

struct HangingSleep {
    cancelled: Arc<AtomicBool>,
}

impl PollSleeper for HangingSleep {
    fn sleep(&self, _duration: Duration) -> impl Future<Output = ()> + Send {
        let token = Token(Arc::clone(&self.cancelled));
        async move {
            let _keep = token;
            pending::<()>().await;
        }
    }
}

fn initial(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn submission() -> UpstreamResponse {
    initial(200, SUBMITTED)
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    future.poll(&mut context)
}

fn clock(step: u64) -> StepClock {
    StepClock {
        millis: AtomicU64::new(0),
        step,
    }
}

#[test]
fn the_monotonic_clock_counts_the_time_since_its_origin() {
    let origin = Instant::now()
        .checked_sub(Duration::from_secs(5))
        .expect("an instant five seconds ago");
    let clock = MonotonicClock { origin };

    assert!(clock.now() >= Duration::from_secs(5));
}

#[tokio::test]
async fn a_ready_poll_returns_the_reply_after_sleeping_through_pending() {
    let script = Script::new(
        vec![
            Reply {
                status: 200,
                body: PENDING,
            },
            Reply {
                status: 200,
                body: READY,
            },
        ],
        Reply {
            status: 200,
            body: PENDING,
        },
    );
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let reply = poller
        .poll(&submission(), &HeaderMap::new())
        .await
        .expect("a ready job returns");
    assert_eq!(reply.status.as_deref(), Some("Ready"));
    assert_eq!(script.urls().len(), 2);
    assert_eq!(
        *sleeper.slept.lock().unwrap_or_else(PoisonError::into_inner),
        vec![Duration::from_millis(1500)]
    );
}

#[tokio::test]
async fn the_initial_status_is_checked_before_the_body_is_parsed() {
    let script = Script::once(200, PENDING);
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&initial(400, "not json"), &HeaderMap::new())
        .await
        .expect_err("a rejected submission fails");
    assert!(matches!(
        &error,
        PollError::InitialStatus { status: 400, .. }
    ));
    assert_eq!(error.status(), Some(400));
    assert_eq!(
        error.to_string(),
        "the BFL submission answered HTTP 400: not json"
    );
    assert!(script.urls().is_empty());
}

#[tokio::test]
async fn an_unparseable_submission_keeps_the_initial_status() {
    let script = Script::once(200, PENDING);
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&initial(201, "not json"), &HeaderMap::new())
        .await
        .expect_err("a body that is not a task fails");
    assert!(matches!(
        &error,
        PollError::InitialDecode { status: 201, .. }
    ));
    assert_eq!(error.status(), Some(201));
    assert!(script.urls().is_empty());
}

#[rstest]
#[case(r#"{"errors":null,"polling_url":"https://gateway.bfl.ai/v1/get_result"}"#)]
#[case(r#"{"errors":[],"polling_url":"https://gateway.bfl.ai/v1/get_result"}"#)]
#[case(r#"{"errors":{},"polling_url":"https://gateway.bfl.ai/v1/get_result"}"#)]
#[case(r#"{"errors":"provider said no","polling_url":"https://gateway.bfl.ai/v1/get_result"}"#)]
#[tokio::test]
async fn a_present_errors_key_rejects_the_submission_even_when_null(#[case] body: &str) {
    let script = Script::once(200, PENDING);
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&initial(200, body), &HeaderMap::new())
        .await
        .expect_err("presence alone rejects");
    assert!(matches!(
        &error,
        PollError::InitialErrors { status: 200, .. }
    ));
    assert!(script.urls().is_empty());
}

#[rstest]
#[case(r#"{"id":"job-1"}"#)]
#[case(r#"{"polling_url":""}"#)]
#[case(r#"{"polling_url":null}"#)]
#[tokio::test]
async fn a_missing_or_empty_polling_url_is_a_500(#[case] body: &str) {
    let script = Script::once(200, PENDING);
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&initial(200, body), &HeaderMap::new())
        .await
        .expect_err("no polling url fails");
    assert!(matches!(&error, PollError::MissingPollingUrl));
    assert_eq!(error.status(), Some(500));
    assert!(script.urls().is_empty());
}

#[rstest]
#[case("http://gateway.bfl.ai/v1/get_result")]
#[case("https://evil.test/v1/get_result")]
#[case("https://bfl.ai.example.test/v1/get_result")]
#[tokio::test]
async fn an_untrusted_polling_url_is_rejected_before_a_get(#[case] polling_url: &str) {
    let script = Script::once(200, PENDING);
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let body = format!(r#"{{"polling_url":"{polling_url}"}}"#);
    let error = poller
        .poll(&initial(200, &body), &HeaderMap::new())
        .await
        .expect_err("an untrusted host fails");
    assert!(matches!(&error, PollError::PollingUrl { .. }));
    assert_eq!(error.status(), Some(502));
    assert!(script.urls().is_empty());
}

#[rstest]
#[case(201)]
#[case(500)]
#[tokio::test]
async fn a_poll_status_other_than_200_fails_with_that_status(#[case] status: u16) {
    let script = Script::once(status, PENDING);
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&submission(), &HeaderMap::new())
        .await
        .expect_err("only 200 is a poll");
    assert!(matches!(&error, PollError::PollStatus { status: found, .. } if *found == status));
    assert_eq!(error.status(), Some(status));
}

#[rstest]
#[case("Error", r#"{"status":"Error"}"#)]
#[case("Failed", r#"{"status":"Failed"}"#)]
#[case("Content Moderated", r#"{"status":"Content Moderated"}"#)]
#[case("Request Moderated", r#"{"status":"Request Moderated"}"#)]
#[tokio::test]
async fn a_terminal_generation_state_fails_with_400(
    #[case] state: &str,
    #[case] body: &'static str,
) {
    let script = Script::once(200, body);
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&submission(), &HeaderMap::new())
        .await
        .expect_err("a terminal state fails");
    assert!(matches!(&error, PollError::Failed { state: found } if found.as_str() == state));
    assert_eq!(error.status(), Some(400));
    assert!(error.to_string().starts_with("Image generation failed: "));
}

#[tokio::test]
async fn the_deadline_is_checked_before_each_get() {
    let script = Script::once(200, PENDING);
    let clock = clock(300_000);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&submission(), &HeaderMap::new())
        .await
        .expect_err("an elapsed deadline ends the poll");
    assert!(matches!(&error, PollError::TimedOut));
    assert_eq!(error.status(), Some(408));
    assert!(script.urls().is_empty());
}

#[tokio::test]
async fn a_deadline_reached_between_gets_still_times_out() {
    let script = Script::once(200, PENDING);
    let clock = clock(299_999);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&submission(), &HeaderMap::new())
        .await
        .expect_err("the second check sees the elapsed deadline");
    assert!(matches!(&error, PollError::TimedOut));
    assert_eq!(script.urls().len(), 1);
}

#[tokio::test]
async fn a_job_still_pending_at_the_last_check_times_out() {
    let script = Script::once(200, PENDING);
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&submission(), &HeaderMap::new())
        .await
        .expect_err("a job pending for 300 seconds fails");
    assert!(matches!(&error, PollError::TimedOut));
    assert_eq!(script.urls().len(), 200);
    assert_eq!(
        sleeper
            .slept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len(),
        200
    );
}

#[tokio::test]
async fn a_poll_decode_failure_keeps_its_shared_error_path() {
    let script = Script::once(200, "not json");
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&submission(), &HeaderMap::new())
        .await
        .expect_err("a body that is not a task fails");
    assert!(matches!(&error, PollError::PollDecode { .. }));
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_poll_transport_failure_keeps_its_shared_error_path() {
    struct Unreachable;

    impl PollTransport for Unreachable {
        async fn get(
            &self,
            _url: Url,
            _headers: HeaderMap,
        ) -> Result<UpstreamResponse, TransportError> {
            Err(TransportError::Closed)
        }
    }

    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &Unreachable,
        clock: &clock,
        sleeper: &sleeper,
    };
    let error = poller
        .poll(&submission(), &HeaderMap::new())
        .await
        .expect_err("a failed poll request fails");
    assert!(matches!(&error, PollError::PollTransport { .. }));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn the_polling_get_carries_only_the_x_key_header(#[case] carrying_key: bool) {
    let script = Script::once(200, READY);
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        "application/json".parse().expect("a header"),
    );
    if carrying_key {
        headers.insert("x-key", "secret".parse().expect("a header"));
    }
    poller
        .poll(&submission(), &headers)
        .await
        .expect("a ready job returns");
    assert_eq!(script.header_names(0), vec![String::from("x-key")]);
    let x_key = script
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .and_then(|(_, headers)| headers.get("x-key").cloned())
        .expect("the poll has an x-key");
    let expected = if carrying_key { "secret" } else { "" };
    assert_eq!(x_key.to_str().expect("a visible key"), expected);
}

#[test]
fn dropping_a_pending_poll_cancels_its_request() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let transport = HangingGet {
        cancelled: Arc::clone(&cancelled),
    };
    let clock = clock(0);
    let sleeper = Recorder {
        clock: &clock,
        slept: Mutex::default(),
    };
    let poller = BflPoller {
        transport: &transport,
        clock: &clock,
        sleeper: &sleeper,
    };
    let mut future = Box::pin(poller.poll(&submission(), &HeaderMap::new()));
    assert!(poll_once(future.as_mut()).is_pending());
    drop(future);
    assert!(cancelled.load(Ordering::SeqCst));
}

#[test]
fn dropping_a_pending_poll_cancels_its_timer() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let script = Script::once(200, PENDING);
    let clock = clock(0);
    let sleeper = HangingSleep {
        cancelled: Arc::clone(&cancelled),
    };
    let poller = BflPoller {
        transport: &script,
        clock: &clock,
        sleeper: &sleeper,
    };
    let mut future = Box::pin(poller.poll(&submission(), &HeaderMap::new()));
    assert!(poll_once(future.as_mut()).is_pending());
    drop(future);
    assert!(cancelled.load(Ordering::SeqCst));
}
