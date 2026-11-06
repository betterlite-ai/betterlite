use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Subscriber;
use tracing::subscriber::set_default;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};

use crate::client::cleanup::failure::ErrorContext;
use crate::client::sessions::error::{McpFailure, SessionError};
use crate::client::sessions::run::{HTTP_CLOSE_TIMEOUT, run_with_session};
use crate::client::sessions::transport::{StdioConfig, TransportType};

use super::fixture::{
    Config, FakeRuntime, InitStep, Plan, Step, TestError, build_client, make_config, make_plan,
};

struct Counts {
    warns: Arc<AtomicUsize>,
    debugs: Arc<AtomicUsize>,
}

impl<Current> Layer<Current> for Counts
where
    Current: Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Current>) {
        match *event.metadata().level() {
            tracing::Level::WARN => {
                self.warns.fetch_add(1, Ordering::Relaxed);
            }
            tracing::Level::DEBUG => {
                self.debugs.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn a_failed_session_logs_a_warning_by_default() {
    let plan = make_plan(|value| value.session_enter = Step::Fail);
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        Config::default(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let warns = Arc::new(AtomicUsize::new(0));
    let debugs = Arc::new(AtomicUsize::new(0));
    let _guard = set_default(tracing_subscriber::registry().with(Counts {
        warns: Arc::clone(&warns),
        debugs: Arc::clone(&debugs),
    }));
    let outcome = run_with_session(&mut client, async |_session| Ok("done"), false).await;
    match outcome {
        Err(SessionError::Runtime(error)) => {
            assert_eq!(*error, TestError::Boom("session enter"));
        }
        Ok(_) => panic!("expected an error"),
        other @ Err(
            SessionError::Protocol(_)
            | SessionError::Stream(_)
            | SessionError::Cancelled
            | SessionError::CleanupTimedOut
            | SessionError::CleanupTask(_)
            | SessionError::ReadTimeout(_)
            | SessionError::Configuration(_),
        ) => {
            panic!("expected the session error, got {other:?}")
        }
    }
    assert_eq!(warns.load(Ordering::Relaxed), 1);
    assert_eq!(debugs.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn quiet_on_error_logs_at_debug() {
    let plan = make_plan(|value| value.session_enter = Step::Fail);
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        Config::default(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let warns = Arc::new(AtomicUsize::new(0));
    let debugs = Arc::new(AtomicUsize::new(0));
    let _guard = set_default(tracing_subscriber::registry().with(Counts {
        warns: Arc::clone(&warns),
        debugs: Arc::clone(&debugs),
    }));
    run_with_session(&mut client, async |_session| Ok("done"), true)
        .await
        .err()
        .unwrap();
    assert_eq!(warns.load(Ordering::Relaxed), 0);
    assert_eq!(debugs.load(Ordering::Relaxed), 1);
}

fn read_timeout_error(context: Option<ErrorContext>) -> SessionError<TestError> {
    SessionError::Protocol(McpFailure {
        code: -32001,
        message: "timed out".to_owned(),
        context,
    })
}

#[tokio::test]
async fn an_sdk_read_timeout_is_normalized() {
    let runtime = FakeRuntime::new(Plan::default());
    let mut client = build_client(
        runtime,
        Config::default(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let warns = Arc::new(AtomicUsize::new(0));
    let debugs = Arc::new(AtomicUsize::new(0));
    let _guard = set_default(tracing_subscriber::registry().with(Counts {
        warns: Arc::clone(&warns),
        debugs: Arc::clone(&debugs),
    }));
    let outcome = run_with_session(
        &mut client,
        async |_session| Err::<(), _>(read_timeout_error(Some(ErrorContext::Timeout))),
        false,
    )
    .await;
    assert!(matches!(outcome, Err(SessionError::ReadTimeout(_))));
    assert_eq!(warns.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn a_relayed_timeout_code_is_not_normalized() {
    let runtime = FakeRuntime::new(Plan::default());
    let mut client = build_client(
        runtime,
        Config::default(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let outcome = run_with_session(
        &mut client,
        async |_session| Err::<(), _>(read_timeout_error(None)),
        false,
    )
    .await;
    match outcome {
        Err(SessionError::Protocol(failure)) => assert_eq!(failure.code, -32_001_i32),
        Ok(()) => panic!("expected an error"),
        other @ Err(
            SessionError::Runtime(_)
            | SessionError::Stream(_)
            | SessionError::Cancelled
            | SessionError::CleanupTimedOut
            | SessionError::CleanupTask(_)
            | SessionError::ReadTimeout(_)
            | SessionError::Configuration(_),
        ) => {
            panic!("expected the relayed protocol error, got {other:?}")
        }
    }
}

#[tokio::test(start_paused = true)]
async fn the_http_client_close_is_bounded_to_one_second() {
    let plan = make_plan(|value| value.http_close = Step::Timeout);
    let runtime = FakeRuntime::new(plan);
    let http_client = runtime.http_client.clone();
    let mut client = build_client(
        runtime,
        make_config(|value| value.transport_type = TransportType::Http),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let started = Instant::now();
    let outcome = run_with_session(&mut client, async |_session| Ok("done"), false).await;
    assert_eq!(outcome.unwrap(), "done");
    assert_eq!(started.elapsed(), HTTP_CLOSE_TIMEOUT);
    assert!(http_client.closed());
}

#[tokio::test]
async fn a_cancelled_http_close_propagates() {
    let plan = make_plan(|value| value.http_close = Step::Cancel);
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        make_config(|value| value.transport_type = TransportType::Http),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let outcome = run_with_session(&mut client, async |_session| Ok("done"), false).await;
    assert!(matches!(outcome, Err(SessionError::Cancelled)));
}

#[tokio::test]
async fn stale_instructions_are_reset_and_replaced_by_the_session() {
    let plan = make_plan(|value| value.instructions = Some("  fresh  ".to_owned()));
    let runtime = FakeRuntime::new(plan);
    let script = runtime.script.clone();
    let mut client = build_client(
        runtime,
        Config::default(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    client.last_initialize_instructions = Some("stale".to_owned());
    run_with_session(&mut client, async |_session| Ok("done"), false)
        .await
        .unwrap();
    assert_eq!(
        client.last_initialize_instructions.as_deref(),
        Some("fresh")
    );
    assert!(script.calls().contains(&"initialize".to_owned()));
}

#[tokio::test]
async fn a_failed_initialize_clears_stale_instructions() {
    let plan = make_plan(|value| value.initialize = InitStep::Fail);
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        Config::default(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    client.last_initialize_instructions = Some("stale".to_owned());
    run_with_session(&mut client, async |_session| Ok("done"), false)
        .await
        .err()
        .unwrap();
    assert!(client.last_initialize_instructions.is_none());
}

#[tokio::test]
async fn a_stdio_session_never_closes_an_http_client() {
    let runtime = FakeRuntime::new(Plan::default());
    let script = runtime.script.clone();
    let mut client = build_client(
        runtime,
        make_config(|value| {
            value.transport_type = TransportType::Stdio;
            value.stdio = Some(StdioConfig {
                command: "npx".to_owned(),
                args: Vec::new(),
                env: Some(BTreeMap::new()),
            });
        }),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    run_with_session(&mut client, async |_session| Ok("done"), false)
        .await
        .unwrap();
    assert!(!script.calls().contains(&"http.close".to_owned()));
}
