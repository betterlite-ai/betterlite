#![cfg(test)]

use std::str;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::mock_response::UsageDefaults;
use betterlite_context::call_context::BillingPolicy;
use betterlite_openai::chat::ChatRequest as ClientChatRequest;
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};
use betterlite_openai::stream::chunk::Chunk;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use rstest::rstest;
use uuid::Uuid;

use super::mock_frames;
use crate::mock::stream::StreamSpec;

use crate::dispatch::error::DispatchError;
use crate::dispatch::log::CallOutcome;
use crate::dispatch::tests::support::{
    FakeChat, MODEL_ID, RecordingLog, RecordingRules, StreamEnd, Trace, chat_request, context,
    deployment,
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

#[tokio::test]
async fn the_rules_read_the_joined_message_text() {
    let (trace, log, rules) = fixtures(None);
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(1))],
        Arc::clone(&rules),
        log,
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let mut request = chat_request("rules-1", MODEL_ID, calls, false);
    request.request.messages = vec![
        ChatMessage {
            role: Role::User,
            content: Some(MessageContent::Text(String::from("hi "))),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            passthrough: IndexMap::new(),
        },
        ChatMessage {
            role: Role::Assistant,
            content: Some(MessageContent::Text(String::from("there"))),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            passthrough: IndexMap::new(),
        },
    ];
    dispatch
        .execute(Operation::Chat(request), &context)
        .await
        .expect("chat call");
    assert_eq!(rules.inputs(), [String::from("hi there")]);
    assert_eq!(trace.steps(), ["rules", "prepare", "complete", "success"]);
}

#[tokio::test]
async fn a_first_message_without_content_skips_the_rules() {
    let (trace, log, rules) = fixtures(None);
    let calls = Arc::new(FakeChat::new(trace.clone(), StreamEnd::Ends));
    let dispatch = Dispatch::new(
        vec![deployment(MODEL_ID, Duration::from_mins(1))],
        Arc::clone(&rules),
        log,
    );
    let context = context(None, SystemTime::UNIX_EPOCH);
    let mut request = chat_request("rules-2", MODEL_ID, calls, false);
    request.request.messages = vec![ChatMessage {
        role: Role::Tool,
        content: None,
        name: None,
        tool_calls: None,
        tool_call_id: Some(String::from("call-0")),
        passthrough: IndexMap::new(),
    }];
    dispatch
        .execute(Operation::Chat(request), &context)
        .await
        .expect("chat call");
    assert!(rules.inputs().is_empty());
    assert_eq!(trace.steps(), ["prepare", "complete", "success"]);
}

const PLAIN: &[u8] = br#"{"model":"g","messages":[]}"#;
const WITH_USAGE: &[u8] =
    br#"{"model":"g","messages":[],"stream":true,"stream_options":{"include_usage":true}}"#;
const DONE: &[u8] = b"data: [DONE]\n\n";

// every frame before the closing [DONE], decoded as the chunk it carries
fn mock_stream_chunks(
    body: &[u8],
    prompt_tokens: Option<u64>,
    choice_count: Option<u32>,
) -> Vec<Chunk> {
    let request = ClientChatRequest::parse(body).expect("mock request");
    let spec = StreamSpec {
        content: "abcdefg",
        choice_count,
        model: "g",
        id: "chatcmpl-1",
        created: 7,
    };
    let usage = UsageDefaults {
        prompt_tokens: 10,
        completion_tokens: 20,
    };
    let mut frames = mock_frames(&spec, &request, prompt_tokens, usage).expect("frames");
    assert_eq!(frames.pop().as_deref(), Some(DONE));
    frames
        .iter()
        .map(|frame| {
            let text = str::from_utf8(frame).expect("utf-8 frame");
            let json = text
                .strip_prefix("data: ")
                .and_then(|rest| rest.strip_suffix("\n\n"))
                .expect("sse frame");
            serde_json::from_str(json).expect("chunk")
        })
        .collect()
}

#[test]
fn mock_frames_slice_the_content_and_keep_the_role_on_the_first_delta() {
    let chunks = mock_stream_chunks(PLAIN, None, None);
    assert_eq!(chunks.len(), 4);
    let deltas: Vec<_> = chunks[..3]
        .iter()
        .map(|chunk| &chunk.choices[0].delta)
        .collect();
    assert_eq!(deltas[0].role.as_deref(), Some("assistant"));
    assert_eq!(deltas[0].content.as_deref(), Some("abc"));
    assert_eq!(deltas[1].role, None);
    assert_eq!(deltas[1].content.as_deref(), Some("def"));
    assert_eq!(deltas[2].role, None);
    assert_eq!(deltas[2].content.as_deref(), Some("g"));
    assert_eq!(chunks[3].choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(chunks[3].id, "chatcmpl-1");
    assert_eq!(chunks[3].created, 7);
}

#[rstest]
#[case::asked_for(WITH_USAGE, None, Some((10, 30)))]
#[case::prompt_count_from_the_plan(PLAIN, Some(7), Some((7, 27)))]
#[case::not_asked_for(PLAIN, None, None)]
fn mock_frames_end_with_a_usage_chunk_only_when_asked_for(
    #[case] body: &[u8],
    #[case] prompt_tokens: Option<u64>,
    #[case] expected: Option<(u64, u64)>,
) {
    let chunks = mock_stream_chunks(body, prompt_tokens, None);
    let usage = chunks[3..].last().and_then(|chunk| chunk.usage.as_ref());
    assert_eq!(
        usage.map(|counts| (counts.prompt_tokens, counts.total_tokens)),
        expected
    );
    assert_eq!(chunks.len(), expected.map_or(4, |_| 5));
    assert!(chunks[..4].iter().all(|chunk| chunk.usage.is_none()));
}

#[test]
fn mock_frames_drop_the_content_chunks_when_no_choice_is_asked_for() {
    let chunks = mock_stream_chunks(PLAIN, None, Some(0));
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].choices[0].finish_reason.as_deref(), Some("stop"));
}
