#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_logging::worker::Admission;
use rstest::rstest;

use super::super::seams::ReportedFailure;
use super::super::slots::{DetachedFailureHook, HookFailure, PostCallFailureHook};
use super::super::{Logged, abort_upstream};
use super::support::{Harness, capture_warnings, failure};
use crate::api::messages_sse_events::stream::MessagesProviderStream;

#[tokio::test]
async fn logging_failures_are_best_effort() {
    let harness = Harness::new(Some(failure("ValueError", "broken")));
    let (captured, _guard) = capture_warnings();
    let logged = harness
        .stream
        .bill_collected_chunks(vec![b"a".to_vec(), b"b".to_vec()], false);
    assert_eq!(logged, None);
    assert!(
        captured
            .text()
            .contains("async_sse_wrapper billing failed after 2 chunks: ValueError(broken)")
    );
    assert_eq!(harness.worker.stats().accepted, 0);
}

#[tokio::test]
async fn billing_success_returns_how_it_was_logged() {
    let harness = Harness::new(None);
    let (captured, _guard) = capture_warnings();
    let logged = harness.stream.bill_collected_chunks(Vec::new(), false);
    assert_eq!(logged, Some(Logged::Enqueued(Admission::Accepted)));
    assert_eq!(captured.text(), "");
}

struct ClosingStream {
    close: Option<Result<(), ReportedFailure>>,
}

#[async_trait]
impl MessagesProviderStream for ClosingStream {
    type Error = ReportedFailure;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ReportedFailure> {
        Ok(None)
    }

    async fn aclose(&mut self) -> Option<Result<(), ReportedFailure>> {
        self.close.clone()
    }
}

#[rstest]
#[case::close_fails(
    Some(Err(failure("ValueError", "closed"))),
    "async_sse_wrapper failed to abort upstream stream: ValueError(closed)"
)]
#[case::close_works(Some(Ok(())), "")]
#[case::close_unsupported(None, "")]
#[tokio::test]
async fn upstream_close_failures_are_warned_not_raised(
    #[case] close: Option<Result<(), ReportedFailure>>,
    #[case] warning: &str,
) {
    let (captured, _guard) = capture_warnings();
    abort_upstream(&mut ClosingStream { close }).await;
    let text = captured.text();
    assert_eq!(text.contains(warning), !warning.is_empty());
    assert_eq!(text.is_empty(), warning.is_empty());
}

struct ScriptedHook {
    outcome: Result<(), ReportedFailure>,
    seen: Mutex<Vec<ReportedFailure>>,
}

#[async_trait]
impl DetachedFailureHook for ScriptedHook {
    async fn on_detached_failure(
        &self,
        exception: &ReportedFailure,
    ) -> Result<(), ReportedFailure> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(exception.clone());
        self.outcome.clone()
    }
}

#[rstest]
#[case::no_hook(None, "", 0)]
#[case::hook_works(Some(Ok(())), "", 1)]
#[case::hook_raises(
    Some(Err(failure("ValueError", "hook"))),
    "async_sse_wrapper detached failure hook raised: ValueError(hook)",
    1
)]
#[tokio::test]
async fn detached_failure_hook_gets_the_original_exception(
    #[case] outcome: Option<Result<(), ReportedFailure>>,
    #[case] warning: &str,
    #[case] expected_calls: usize,
) {
    let harness = Harness::new(None);
    let original = failure("RuntimeError", "upstream");
    let hook = outcome.map(|outcome| {
        Arc::new(ScriptedHook {
            outcome,
            seen: Mutex::new(Vec::new()),
        })
    });
    harness
        .slots
        .fields
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .detached_failure_hook = hook
        .clone()
        .map(|armed| -> Arc<dyn DetachedFailureHook> { armed });
    let (captured, _guard) = capture_warnings();
    harness.stream.fire_detached_failure_hook(&original).await;
    let seen = hook.map_or_else(Vec::new, |armed| {
        armed
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    });
    assert_eq!(seen, vec![original; expected_calls]);
    assert_eq!(captured.text().contains(warning), !warning.is_empty());
    assert_eq!(captured.text().is_empty(), warning.is_empty());
}

struct ScriptedPostCall {
    outcome: Result<(), HookFailure>,
    seen: Mutex<Vec<ReportedFailure>>,
}

#[async_trait]
impl PostCallFailureHook for ScriptedPostCall {
    async fn post_call_failure(&self, original: &ReportedFailure) -> Result<(), HookFailure> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(original.clone());
        self.outcome.clone()
    }
}

#[rstest]
#[case::hook_works(Ok(()), "")]
#[case::http_exception_is_dropped(Err(HookFailure::Http), "")]
#[case::other_failure_is_warned(
    Err(HookFailure::Other(failure("KeyError", "key"))),
    "async_sse_wrapper detached failure hook raised: KeyError(key)"
)]
#[tokio::test]
async fn armed_hook_reaches_post_call_failure_hook(
    #[case] outcome: Result<(), HookFailure>,
    #[case] warning: &str,
) {
    let harness = Harness::new(None);
    let post_call = Arc::new(ScriptedPostCall {
        outcome,
        seen: Mutex::new(Vec::new()),
    });
    harness
        .slots
        .arm_failure_hook(Arc::<ScriptedPostCall>::clone(&post_call));
    let original = failure("RuntimeError", "upstream");
    let (captured, _guard) = capture_warnings();
    harness.stream.fire_detached_failure_hook(&original).await;
    let seen = post_call
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(*seen, vec![original]);
    assert_eq!(captured.text().contains(warning), !warning.is_empty());
    assert_eq!(captured.text().is_empty(), warning.is_empty());
}
