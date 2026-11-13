#![cfg(test)]

use std::str;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_context::call_context::BillingPolicy;
use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use rstest::rstest;
use uuid::Uuid;

use crate::dispatch::error::DispatchError;
use crate::dispatch::log::CallOutcome;
use crate::dispatch::tests::support::{
    FakeChat, MODEL_ID, RecordingLog, RecordingRules, StreamEnd, Trace, chat_request,
    chat_request_from, context, deployment,
};
use crate::dispatch::{Dispatch, Entry, NoRules};
use crate::operation::chat::{ChatFailure, ChatResult};
use crate::operation::{CallResult, Operation};
use crate::rules::RuleDecision;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    LifecycleOrder,
    RejectedRule,
    ExpiredDeadline,
    DeadlineCapsTimeout,
    MissingDeployment,
    InvalidControl,
    ChildContext,
    Concurrent,
}

#[rstest]
#[case::lifecycle_order(Scenario::LifecycleOrder)]
#[case::rejected_rule(Scenario::RejectedRule)]
#[case::expired_deadline(Scenario::ExpiredDeadline)]
#[case::deadline_caps_timeout(Scenario::DeadlineCapsTimeout)]
#[case::missing_deployment(Scenario::MissingDeployment)]
#[case::invalid_control(Scenario::InvalidControl)]
#[case::child_context(Scenario::ChildContext)]
#[case::concurrent(Scenario::Concurrent)]
#[tokio::test]
async fn call_dispatch_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::LifecycleOrder => lifecycle_order().await,
        Scenario::RejectedRule => rejected_rule().await,
        Scenario::ExpiredDeadline => {
            refused_before_transport(Some(SystemTime::UNIX_EPOCH), MODEL_ID, |error| {
                matches!(error, DispatchError::DeadlineExpired { .. })
            })
            .await;
        }
        Scenario::DeadlineCapsTimeout => deadline_caps_timeout().await,
        Scenario::MissingDeployment => {
            refused_before_transport(None, "missing", |error| {
                matches!(error, DispatchError::MissingDeployment { .. })
            })
            .await;
        }
        Scenario::InvalidControl => invalid_control().await,
        Scenario::ChildContext => child_context().await,
        Scenario::Concurrent => concurrent().await,
    }
}

fn fixtures(decision: Option<RuleDecision>) -> (Trace, Arc<RecordingLog>, Arc<RecordingRules>) {
    let trace = Trace::default();
    let log = Arc::new(RecordingLog::new(trace.clone()));
    let rules = Arc::new(RecordingRules::new(trace.clone(), decision));
    (trace, log, rules)
}

async fn lifecycle_order() {
    let (trace, log, rules) = fixtures(Some(RuleDecision::Boolean(true)));
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        rules,
        Arc::clone(&log),
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let result = dispatch
        .execute(
            Operation::Chat(chat_request("call-1", MODEL_ID, Arc::clone(&calls), false)),
            &context,
        )
        .await
        .expect("chat call");
    assert!(matches!(
        result,
        CallResult::Chat(ChatResult::Completion(_))
    ));
    assert_eq!(trace.steps(), ["rules", "prepare", "complete", "success"]);
    assert!(!calls.seen()[0].stream);
    let recorded = log.calls();
    assert!(!recorded[0].privacy);
}

async fn rejected_rule() {
    let (trace, log, rules) = fixtures(Some(RuleDecision::Boolean(false)));
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        rules,
        Arc::clone(&log),
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let Err(error) = dispatch
        .execute(
            Operation::Chat(chat_request("call-2", MODEL_ID, Arc::clone(&calls), false)),
            &context,
        )
        .await
    else {
        panic!("a rejected rule must fail the call");
    };
    assert!(matches!(error, DispatchError::Rules { .. }));
    assert_eq!(trace.steps(), ["rules", "failure"]);
    assert!(calls.seen().is_empty());
    let recorded = log.calls();
    assert_eq!(recorded.len(), 1);
    assert!(
        recorded[0]
            .message
            .as_deref()
            .is_some_and(|message| message.contains("rule"))
    );
}

async fn deadline_caps_timeout() {
    let (trace, log, rules) = fixtures(None);
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        rules,
        log,
    );
    let deadline = SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(5))
        .expect("deadline");
    let context = context(Some(deadline), SystemTime::UNIX_EPOCH);
    dispatch
        .execute(
            Operation::Chat(chat_request("call-4", MODEL_ID, Arc::clone(&calls), false)),
            &context,
        )
        .await
        .expect("bounded call");
    assert_eq!(calls.seen()[0].timeout, Duration::from_secs(5));
    assert!(!calls.seen()[0].stream);
}

async fn refused_before_transport(
    deadline: Option<SystemTime>,
    model_id: &str,
    expected: fn(&DispatchError) -> bool,
) {
    let (trace, log, rules) = fixtures(None);
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        rules,
        log,
    );
    let context = context(deadline, SystemTime::UNIX_EPOCH);
    let Err(error) = dispatch
        .execute(
            Operation::Chat(chat_request("call-3", model_id, Arc::clone(&calls), false)),
            &context,
        )
        .await
    else {
        panic!("the call must fail before transport");
    };
    assert!(expected(&error), "unexpected error: {error}");
    assert_eq!(trace.steps(), ["failure"]);
    assert!(calls.seen().is_empty());
}

async fn invalid_control() {
    let (trace, log, rules) = fixtures(None);
    let mut fake = FakeChat::new(trace.clone(), StreamEnd::Ends);
    fake.chunk_size = Some(0);
    let calls = Arc::new(fake);
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        rules,
        log,
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let Err(error) = dispatch
        .execute(
            Operation::Chat(chat_request("call-6", MODEL_ID, Arc::clone(&calls), false)),
            &context,
        )
        .await
    else {
        panic!("an invalid control option must fail the call");
    };
    assert!(matches!(
        error,
        DispatchError::Chat {
            source: ChatFailure::InvalidControl { .. },
        }
    ));
    assert_eq!(trace.steps(), ["rules", "prepare", "failure"]);
    assert!(calls.seen().is_empty());
}

async fn child_context() {
    let (trace, log, rules) = fixtures(None);
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(100))],
        rules,
        Arc::clone(&log),
    );
    let parent = context(None, SystemTime::UNIX_EPOCH);
    let child = parent.child(Uuid::from_u128(7));
    dispatch
        .execute(
            Operation::Chat(chat_request("child-call", MODEL_ID, calls, false)),
            &child,
        )
        .await
        .expect("child call");
    let recorded = log.calls();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].billing, BillingPolicy::Suppress);
    assert_eq!(recorded[0].call_id, "child-call");
    assert_eq!(recorded[0].entry, Entry::Chat);
}

async fn run_chat(
    dispatch: &Dispatch,
    call_id: &str,
    model_id: &str,
    calls: Arc<FakeChat>,
) -> Result<CallResult, DispatchError> {
    let context = context(None, SystemTime::UNIX_EPOCH);
    dispatch
        .execute(
            Operation::Chat(chat_request(call_id, model_id, calls, false)),
            &context,
        )
        .await
}

async fn concurrent() {
    let trace = Trace::default();
    let first_calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let second_calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let log = Arc::new(RecordingLog::new(trace));
    let dispatch = Dispatch::new(
        vec![
            deployment(MODEL_ID, Duration::from_secs(10)),
            deployment("dep-2", Duration::from_secs(20)),
        ],
        Arc::new(NoRules),
        Arc::clone(&log),
    );
    let (first, second) = tokio::join!(
        run_chat(&dispatch, "call-a", MODEL_ID, Arc::clone(&first_calls)),
        run_chat(&dispatch, "call-b", "dep-2", Arc::clone(&second_calls)),
    );
    let _first = first.expect("first call");
    let _second = second.expect("second call");
    assert_eq!(first_calls.seen()[0].model_id, MODEL_ID);
    assert_eq!(second_calls.seen()[0].model_id, "dep-2");
    let recorded = log.calls();
    assert_eq!(recorded.len(), 2);
    assert!(
        recorded
            .iter()
            .any(|call| call.call_id == "call-a" && call.entry == Entry::Chat)
    );
    assert!(recorded.iter().any(|call| call.call_id == "call-b"));
}

#[tokio::test]
async fn a_completed_stream_logs_success_only_after_it_ends() {
    let trace = Trace::default();
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let log = Arc::new(RecordingLog::new(trace));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_secs(10))],
        Arc::new(NoRules),
        Arc::clone(&log),
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let result = dispatch
        .execute(
            Operation::Chat(chat_request("stream-1", MODEL_ID, Arc::clone(&calls), true)),
            &context,
        )
        .await
        .expect("stream call");
    assert!(calls.seen()[0].stream);
    let CallResult::Chat(ChatResult::Stream(answer)) = result else {
        panic!("a streamed call answers a stream");
    };
    assert!(log.calls().is_empty());
    let mut body = answer.body;
    let mut chunks = 0_usize;
    while let Some(chunk) = body.next().await {
        chunk.expect("stream chunk");
        chunks = chunks.saturating_add(1);
    }
    assert_eq!(chunks, 2);
    let recorded = log.calls();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].outcome, CallOutcome::Succeeded);
}

#[tokio::test]
async fn a_dropped_stream_logs_cancellation_once() {
    let trace = Trace::default();
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Pending));
    let log = Arc::new(RecordingLog::new(trace));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_secs(10))],
        Arc::new(NoRules),
        Arc::clone(&log),
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let result = dispatch
        .execute(
            Operation::Chat(chat_request("stream-2", MODEL_ID, calls, true)),
            &context,
        )
        .await
        .expect("stream call");
    let CallResult::Chat(ChatResult::Stream(answer)) = result else {
        panic!("a streamed call answers a stream");
    };
    let mut body = answer.body;
    let first = body.next().await.expect("first chunk").expect("chunk");
    assert_eq!(str::from_utf8(&first).expect("utf8"), "chunk-one");
    drop(body);
    let recorded = log.calls();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].outcome, CallOutcome::Cancelled);
}

const RULES_RAN: &[&str] = &["rules", "prepare", "complete", "success"];
const RULES_SKIPPED: &[&str] = &["prepare", "complete", "success"];

#[rstest]
#[case::joined_text(
    r#"{"model":"g","messages":[{"role":"user","content":"hi "},{"role":"assistant","content":"there"}]}"#,
    &["hi there"],
    RULES_RAN
)]
#[case::text_parts_are_not_joined(
    r#"{"model":"g","messages":[{"role":"user","content":"a"},{"role":"user","content":[{"type":"text","text":"b"}]},{"role":"user","content":"c"}]}"#,
    &["ac"],
    RULES_RAN
)]
#[case::explicit_null_content(
    r#"{"model":"g","messages":[{"role":"user","content":null}]}"#,
    &[""],
    RULES_RAN
)]
#[case::later_message_without_content(
    r#"{"model":"g","messages":[{"role":"user","content":"hi"},{"role":"assistant","tool_calls":[{"id":"c","type":"function","function":{"name":"f","arguments":"{}"}}]}]}"#,
    &["hi"],
    RULES_RAN
)]
#[case::first_message_omits_content(
    r#"{"model":"g","messages":[{"role":"assistant","tool_calls":[{"id":"c","type":"function","function":{"name":"f","arguments":"{}"}}]},{"role":"tool","tool_call_id":"c","content":"result"}]}"#,
    &[],
    RULES_SKIPPED
)]
#[case::no_messages(r#"{"model":"g","messages":[]}"#, &[], RULES_SKIPPED)]
#[tokio::test]
async fn the_rules_run_only_when_the_first_message_has_a_content_key(
    #[case] body: &str,
    #[case] inputs: &[&str],
    #[case] steps: &[&str],
) {
    let (trace, log, rules) = fixtures(None);
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(1))],
        Arc::clone(&rules),
        log,
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let request = chat_request_from(body.as_bytes(), "rules-1", MODEL_ID, calls);
    dispatch
        .execute(Operation::Chat(request), &context)
        .await
        .expect("chat call");
    assert_eq!(rules.inputs(), inputs);
    assert_eq!(trace.steps(), steps);
}

#[tokio::test]
async fn a_failed_stream_logs_failure_once_and_a_later_drop_logs_nothing() {
    let trace = Trace::default();
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Fails));
    let log = Arc::new(RecordingLog::new(trace));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_secs(10))],
        Arc::new(NoRules),
        Arc::clone(&log),
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let result = dispatch
        .execute(
            Operation::Chat(chat_request("stream-3", MODEL_ID, calls, true)),
            &context,
        )
        .await
        .expect("stream call");
    let CallResult::Chat(ChatResult::Stream(answer)) = result else {
        panic!("a streamed call answers a stream");
    };
    let mut body = answer.body;
    body.next().await.expect("first chunk").expect("chunk");
    assert!(log.calls().is_empty());
    let failure = body.next().await.expect("second item");
    assert!(matches!(failure, Err(HttpError::Timeout)));
    let recorded = log.calls();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].outcome, CallOutcome::Failed);
    assert_eq!(
        recorded[0].message.as_deref(),
        Some("the request timed out")
    );
    body.next()
        .await
        .expect("chunk after the error")
        .expect("chunk");
    assert!(body.next().await.is_none());
    drop(body);
    assert_eq!(log.calls().len(), 1);
}
