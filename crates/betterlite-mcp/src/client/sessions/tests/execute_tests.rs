use std::future::pending;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::client::sessions::error::SessionError;
use crate::client::sessions::execute::{SESSION_CLEANUP_TIMEOUT, execute_session_operation};
use crate::client::sessions::stream::{
    MessageHandler, StreamFailure, StreamFailureKind, StreamMessage,
};

use super::fixture::{
    FakeRuntime, InitStep, Plan, Step, TestError, base_config, build_client, make_plan,
    manual_contexts, protocol_error,
};

#[tokio::test]
async fn a_stream_failure_replaces_a_protocol_error() {
    let runtime = FakeRuntime::new(Plan::default());
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome = execute_session_operation(
        &mut client,
        contexts,
        async |session| -> Result<(), SessionError<TestError>> {
            let _failed = session
                .handler
                .handle(StreamMessage::Failure(StreamFailure::Io(Arc::new(
                    TestError::Boom("stream"),
                ))));
            Err(protocol_error(-32000))
        },
    )
    .await;
    match outcome {
        Err(SessionError::Stream(failure)) => {
            assert_eq!(failure.kind(), StreamFailureKind::Io);
            assert_eq!(*failure.source_error().as_ref(), TestError::Boom("stream"));
        }
        Ok(()) => panic!("expected a stream error"),
        other @ Err(
            SessionError::Protocol(_)
            | SessionError::Runtime(_)
            | SessionError::Cancelled
            | SessionError::CleanupTimedOut
            | SessionError::CleanupTask(_)
            | SessionError::ReadTimeout(_)
            | SessionError::Configuration(_),
        ) => {
            panic!("expected the stream error, got {other:?}")
        }
    }
}

#[tokio::test]
async fn a_stream_failure_during_initialize_replaces_the_protocol_error() {
    let plan = make_plan(|value| value.initialize = InitStep::StreamFailure);
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome =
        execute_session_operation(&mut client, contexts, async |_session| Ok("done")).await;
    match outcome {
        Err(SessionError::Stream(failure)) => {
            assert_eq!(failure.kind(), StreamFailureKind::Http);
            assert_eq!(*failure.source_error().as_ref(), TestError::Status(401));
        }
        Ok(_) => panic!("expected a stream error"),
        other @ Err(
            SessionError::Protocol(_)
            | SessionError::Runtime(_)
            | SessionError::Cancelled
            | SessionError::CleanupTimedOut
            | SessionError::CleanupTask(_)
            | SessionError::ReadTimeout(_)
            | SessionError::Configuration(_),
        ) => {
            panic!("expected the stream error, got {other:?}")
        }
    }
}

#[tokio::test]
async fn a_stream_failure_does_not_replace_an_application_error() {
    let runtime = FakeRuntime::new(Plan::default());
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome = execute_session_operation(&mut client, contexts, async |session| {
        let _failed = session
            .handler
            .handle(StreamMessage::Failure(StreamFailure::ValueError(Arc::new(
                TestError::Boom("stream"),
            ))));
        Err::<(), _>(SessionError::Runtime(Arc::new(TestError::Status(500))))
    })
    .await;
    match outcome {
        Err(SessionError::Runtime(error)) => assert_eq!(*error, TestError::Status(500)),
        Ok(()) => panic!("expected an error"),
        other @ Err(
            SessionError::Protocol(_)
            | SessionError::Stream(_)
            | SessionError::Cancelled
            | SessionError::CleanupTimedOut
            | SessionError::CleanupTask(_)
            | SessionError::ReadTimeout(_)
            | SessionError::Configuration(_),
        ) => {
            panic!("expected the application error, got {other:?}")
        }
    }
}

#[test]
fn the_handler_keeps_the_first_stream_failure_and_fails_the_call() {
    let handler: MessageHandler<TestError> = MessageHandler::default();
    assert!(
        handler
            .handle(StreamMessage::Failure(StreamFailure::Io(Arc::new(
                TestError::Boom("first"),
            ))))
            .is_err()
    );
    assert!(
        handler
            .handle(StreamMessage::Failure(StreamFailure::Http(Arc::new(
                TestError::Status(401),
            ))))
            .is_err()
    );
    handler.handle(StreamMessage::Notification).unwrap();
    let recorded = handler
        .recorded
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .expect("a stream failure is recorded");
    assert_eq!(recorded.kind(), StreamFailureKind::Io);
    assert_eq!(*recorded.source_error().as_ref(), TestError::Boom("first"));
}

#[tokio::test]
async fn session_cleanup_failure_keeps_the_operation_error() {
    let plan = make_plan(|value| value.session_exit = Step::Fail);
    let runtime = FakeRuntime::new(plan);
    let script = runtime.script.clone();
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome = execute_session_operation(&mut client, contexts, async |_session| {
        Err::<(), _>(protocol_error(-32001))
    })
    .await;
    assert!(matches!(outcome, Err(SessionError::Protocol(_))));
    assert!(script.calls().contains(&"session.exit".to_owned()));
}

#[tokio::test(start_paused = true)]
async fn session_cleanup_after_success_is_bounded_to_five_seconds() {
    let plan = make_plan(|value| value.session_exit = Step::Timeout);
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let started = Instant::now();
    let outcome =
        execute_session_operation(&mut client, contexts, async |_session| Ok("done")).await;
    assert!(matches!(outcome, Err(SessionError::CleanupTimedOut)));
    assert_eq!(started.elapsed(), SESSION_CLEANUP_TIMEOUT);
}

#[tokio::test(start_paused = true)]
async fn transport_cleanup_is_bounded_to_five_seconds() {
    let plan = make_plan(|value| value.transport_exit = Step::Timeout);
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let started = Instant::now();
    let outcome =
        execute_session_operation(&mut client, contexts, async |_session| Ok("done")).await;
    assert!(matches!(outcome, Err(SessionError::CleanupTimedOut)));
    assert_eq!(started.elapsed(), SESSION_CLEANUP_TIMEOUT);
}

#[tokio::test]
async fn transport_exit_is_skipped_when_the_transport_never_entered() {
    let plan = make_plan(|value| value.transport_enter = Step::Fail);
    let runtime = FakeRuntime::new(plan);
    let script = runtime.script.clone();
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome =
        execute_session_operation(&mut client, contexts, async |_session| Ok("done")).await;
    match outcome {
        Err(SessionError::Runtime(error)) => {
            assert_eq!(*error, TestError::Boom("transport enter"));
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
            panic!("expected the transport error, got {other:?}")
        }
    }
    let calls = script.calls();
    assert!(!calls.contains(&"transport.exit".to_owned()));
    assert!(!calls.contains(&"session.enter".to_owned()));
}

#[tokio::test]
async fn a_session_enter_failure_skips_the_session_exit() {
    let plan = make_plan(|value| value.session_enter = Step::Fail);
    let runtime = FakeRuntime::new(plan);
    let script = runtime.script.clone();
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome =
        execute_session_operation(&mut client, contexts, async |_session| Ok("done")).await;
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
    let calls = script.calls();
    assert!(calls.contains(&"session.enter".to_owned()));
    assert!(!calls.contains(&"session.exit".to_owned()));
    assert!(calls.contains(&"transport.exit".to_owned()));
}

#[tokio::test]
async fn a_transport_failure_after_cancellation_becomes_the_root_cause() {
    let plan = make_plan(|value| value.transport_exit = Step::Fail);
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome = execute_session_operation(&mut client, contexts, async |_session| {
        Err::<(), _>(SessionError::Cancelled)
    })
    .await;
    match outcome {
        Err(SessionError::Runtime(error)) => {
            assert_eq!(*error, TestError::Boom("transport exit"));
        }
        Ok(()) => panic!("expected an error"),
        other @ Err(
            SessionError::Protocol(_)
            | SessionError::Stream(_)
            | SessionError::Cancelled
            | SessionError::CleanupTimedOut
            | SessionError::CleanupTask(_)
            | SessionError::ReadTimeout(_)
            | SessionError::Configuration(_),
        ) => {
            panic!("expected the root cause, got {other:?}")
        }
    }
}

#[tokio::test]
async fn a_recorded_cleanup_error_is_not_the_root_cause() {
    let plan = make_plan(|value| value.transport_exit = Step::RecordedFailure);
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, true);
    let outcome = execute_session_operation(&mut client, contexts, async |_session| {
        Err::<(), _>(SessionError::Cancelled)
    })
    .await;
    assert!(matches!(outcome, Err(SessionError::Cancelled)));
}

#[tokio::test]
async fn the_http_cleanup_scope_is_shielded_and_reset_after_execution() {
    let runtime = FakeRuntime::new(Plan::default());
    let http_client = runtime.http_client.clone();
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, true);
    execute_session_operation(&mut client, contexts, async |_session| Ok("done"))
        .await
        .unwrap();
    let scopes = http_client.scopes();
    assert_eq!(scopes.len(), 2);
    assert!(scopes[0].shield);
    assert!(scopes[1].shield);
    assert_eq!(
        scopes
            .first()
            .unwrap()
            .deadline
            .checked_duration_since(Instant::now())
            .unwrap(),
        SESSION_CLEANUP_TIMEOUT
    );
    assert_eq!(http_client.finished(), 1);
}

#[tokio::test]
async fn instructions_are_trimmed_and_stored() {
    let plan = make_plan(|value| value.instructions = Some("  hello world  ".to_owned()));
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    execute_session_operation(&mut client, contexts, async |_session| Ok("done"))
        .await
        .unwrap();
    assert_eq!(
        client.last_initialize_instructions.as_deref(),
        Some("hello world")
    );
}

#[tokio::test]
async fn empty_instructions_become_none() {
    let plan = make_plan(|value| value.instructions = Some("   ".to_owned()));
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    execute_session_operation(&mut client, contexts, async |_session| Ok("done"))
        .await
        .unwrap();
    assert!(client.last_initialize_instructions.is_none());
}

#[tokio::test(start_paused = true)]
async fn caller_cancellation_during_the_operation_propagates() {
    let runtime = FakeRuntime::new(Plan::default());
    let script = runtime.script.clone();
    let cancelled = CancellationToken::new();
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        cancelled.clone(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let trigger = cancelled.clone();
    let timer = TaskTracker::new();
    timer.spawn(async move {
        sleep(Duration::from_secs(1)).await;
        trigger.cancel();
    });
    let outcome = execute_session_operation(&mut client, contexts, async |_session| {
        pending::<Result<&'static str, SessionError<TestError>>>().await
    })
    .await;
    assert!(matches!(outcome, Err(SessionError::Cancelled)));
    assert!(script.calls().contains(&"session.exit".to_owned()));
    timer.close();
    timer.wait().await;
}

#[tokio::test]
async fn a_pre_cancelled_caller_never_opens_the_transport() {
    let runtime = FakeRuntime::new(Plan::default());
    let script = runtime.script.clone();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let mut client = build_client(runtime, base_config(), TaskTracker::new(), cancelled);
    let contexts = manual_contexts(&client.runtime, false);
    let outcome =
        execute_session_operation(&mut client, contexts, async |_session| Ok("done")).await;
    assert!(matches!(outcome, Err(SessionError::Cancelled)));
    let calls = script.calls();
    assert!(!calls.contains(&"transport.enter".to_owned()));
    assert!(!calls.contains(&"transport.exit".to_owned()));
}

#[tokio::test]
async fn a_cancelled_session_exit_lets_the_transport_failure_become_the_root_cause() {
    let plan = make_plan(|value| {
        value.session_exit = Step::Cancel;
        value.transport_exit = Step::Fail;
    });
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome =
        execute_session_operation(&mut client, contexts, async |_session| Ok("done")).await;
    match outcome {
        Err(SessionError::Runtime(error)) => {
            assert_eq!(*error, TestError::Boom("transport exit"));
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
            panic!("expected the transport root cause, got {other:?}")
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_timed_out_session_exit_lets_the_transport_failure_become_the_root_cause() {
    let plan = make_plan(|value| {
        value.session_exit = Step::Timeout;
        value.transport_exit = Step::Fail;
    });
    let runtime = FakeRuntime::new(plan);
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome =
        execute_session_operation(&mut client, contexts, async |_session| Ok("done")).await;
    match outcome {
        Err(SessionError::Runtime(error)) => {
            assert_eq!(*error, TestError::Boom("transport exit"));
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
            panic!("expected the transport root cause, got {other:?}")
        }
    }
}

#[tokio::test]
async fn a_failed_initialize_skips_the_operation() {
    let plan = make_plan(|value| value.initialize = InitStep::Fail);
    let runtime = FakeRuntime::new(plan);
    let script = runtime.script.clone();
    let mut client = build_client(
        runtime,
        base_config(),
        TaskTracker::new(),
        CancellationToken::new(),
    );
    let contexts = manual_contexts(&client.runtime, false);
    let outcome = execute_session_operation(&mut client, contexts, async |_session| {
        Ok::<&'static str, SessionError<TestError>>("unreachable")
    })
    .await;
    assert!(matches!(outcome, Err(SessionError::Runtime(_))));
    assert!(script.calls().contains(&"session.exit".to_owned()));
}
