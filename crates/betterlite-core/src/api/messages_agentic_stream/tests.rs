#![cfg(test)]

use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt::Debug;
use std::fs;
use std::future::{Future, poll_fn};
use std::path::Path;
use std::pin::Pin;
use std::str::from_utf8;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use async_trait::async_trait;
use betterlite_context::agentic_callback::AgenticLogging;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_models::decisions::Json;
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::Event;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, Subscriber, set_default};
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::interceptor::{
    AgenticFollowUp, AgenticHookFuture, AgenticInterceptor, MessagesFakeFollowUp,
};
use super::stream::MessagesAgenticStream;
use super::{
    HoldBackPolicy, KEEPALIVE_PING, MessagesAgenticStreamInput, MessagesAgenticStreamResult,
    PING_INTERVAL_SECONDS, decode_messages_agentic_stream, is_server_fulfilled_tool_leak_error,
};
use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_fake_stream::{MessagesFakeStream, MessagesFakeStreamError};
use crate::api::messages_sse_events::stream::MessagesProviderStream;
use crate::meta_muse_realtime::request::Sleep;

mod limits;
mod pacing;
mod rebuild;

type EffectLog = Recorder<String>;
type DelayLog = Recorder<u64>;

// a stream that yields more than this never ends, so a drain loop fails instead of hanging
const MAX_DRAINED: usize = 64;

// ordered side effects of the scripted streams and the interceptor, as the oracle records them
#[derive(Clone, Default)]
struct Recorder<T>(Arc<Mutex<Vec<T>>>);

impl<T> Recorder<T> {
    fn push(&self, value: T) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
    }

    fn snapshot(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Recorder<String> {
    fn record(&self, event: &str) {
        self.push(event.to_owned());
    }

    fn count(&self, event: &str) -> usize {
        self.snapshot()
            .iter()
            .filter(|entry| entry.as_str() == event)
            .count()
    }
}

// every tracing event as "LEVEL message", in the order the iterator logged them
impl<Inner: Subscriber> Layer<Inner> for Recorder<String> {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        let mut message = MessageText::default();
        event.record(&mut message);
        self.record(&format!("{} {}", event.metadata().level(), message.0));
    }
}

#[derive(Default)]
struct MessageText(String);

impl Visit for MessageText {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

fn capture_logs() -> (EffectLog, DefaultGuard) {
    let logs = EffectLog::default();
    let guard = set_default(registry().with(logs.clone()));
    (logs, guard)
}

// what the interceptor was handed, so a test can prove the rebuilt response reached it
#[derive(Clone, Default)]
struct HookSeen(Arc<Mutex<Vec<(Json, String)>>>);

impl HookSeen {
    fn push(&self, response: &Json, model: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((response.clone(), model.to_owned()));
    }

    fn entries(&self) -> Vec<(Json, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

// a gate the test opens when it wants the pending read or hook to finish
async fn wait_for(gate: &Arc<AtomicBool>) {
    let gate = Arc::clone(gate);
    poll_fn(move |_context| {
        if gate.load(Ordering::SeqCst) {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
}

fn immediate_sleep() -> Sleep {
    Arc::new(|_seconds: f64| -> BoxFuture<'static, ()> { Box::pin(async {}) })
}

fn recorded_sleep(delays: &DelayLog) -> Sleep {
    let recorded = delays.clone();
    Arc::new(move |seconds: f64| -> BoxFuture<'static, ()> {
        recorded.push(seconds.to_bits());
        Box::pin(async {})
    })
}

struct GatedHook<FollowUp> {
    gate: Arc<AtomicBool>,
    outcome: Option<Result<MessagesAgenticStreamResult<FollowUp>, AgenticHookError>>,
    effects: EffectLog,
}

impl<FollowUp> Future for GatedHook<FollowUp>
where
    FollowUp: Unpin,
{
    type Output = Result<MessagesAgenticStreamResult<FollowUp>, AgenticHookError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if !this.gate.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        match this.outcome.take() {
            Some(outcome) => Poll::Ready(outcome),
            None => Poll::Pending,
        }
    }
}

impl<FollowUp> Drop for GatedHook<FollowUp> {
    fn drop(&mut self) {
        if self.outcome.is_some() {
            self.effects.record("hook-cancelled");
        }
    }
}

enum HookReply {
    Empty,
    // the follow-up frames and the gate that holds each of its reads, None when they are ready at once
    Stream(Vec<Vec<u8>>, Option<Arc<AtomicBool>>),
    Message(Json),
    Unexpected(&'static str),
    Failed(String),
}

struct ScriptedInterceptor {
    gate: Arc<AtomicBool>,
    reply: Mutex<Option<HookReply>>,
    effects: EffectLog,
    seen: HookSeen,
}

impl AgenticInterceptor for ScriptedInterceptor {
    type FollowUp = ScriptedFollowUp;

    fn intercept(
        &self,
        response: &Json,
        input: &MessagesAgenticStreamInput,
    ) -> AgenticHookFuture<Self::FollowUp> {
        self.effects.record("hook-start");
        self.seen.push(response, &input.model);
        let reply = self
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .expect("the interceptor runs once");
        let outcome = reply_outcome(reply, &self.effects);
        Box::pin(GatedHook {
            gate: Arc::clone(&self.gate),
            outcome: Some(outcome),
            effects: self.effects.clone(),
        })
    }
}

fn reply_outcome(
    reply: HookReply,
    effects: &EffectLog,
) -> Result<MessagesAgenticStreamResult<ScriptedFollowUp>, AgenticHookError> {
    match reply {
        HookReply::Empty => Ok(MessagesAgenticStreamResult::NoFollowUp),
        HookReply::Stream(frames, gate) => {
            Ok(MessagesAgenticStreamResult::Stream(ScriptedFollowUp {
                frames: VecDeque::from(frames),
                gate,
                effects: effects.clone(),
                labels: ("followup-read", "followup-end"),
            }))
        }
        HookReply::Message(message) => Ok(MessagesAgenticStreamResult::Message(message)),
        HookReply::Unexpected(type_name) => Ok(MessagesAgenticStreamResult::Unexpected {
            type_name: type_name.to_owned(),
        }),
        HookReply::Failed(message) => Err(AgenticHookError::Failed {
            callback: "scripted-hook".to_owned(),
            message,
        }),
    }
}

fn resolved_interceptor(
    reply: HookReply,
    effects: &EffectLog,
    seen: &HookSeen,
) -> ScriptedInterceptor {
    ScriptedInterceptor {
        gate: Arc::new(AtomicBool::new(true)),
        reply: Mutex::new(Some(reply)),
        effects: effects.clone(),
        seen: seen.clone(),
    }
}

struct ScriptedFollowUp {
    frames: VecDeque<Vec<u8>>,
    gate: Option<Arc<AtomicBool>>,
    effects: EffectLog,
    labels: (&'static str, &'static str),
}

async fn next_frame(
    frames: &mut VecDeque<Vec<u8>>,
    gate: &Option<Arc<AtomicBool>>,
    effects: &EffectLog,
    labels: (&str, &str),
) -> Result<Option<Vec<u8>>, Infallible> {
    if let Some(gate) = gate {
        wait_for(gate).await;
    }
    let frame = frames.pop_front();
    effects.record(if frame.is_some() { labels.0 } else { labels.1 });
    Ok(frame)
}

#[async_trait]
impl MessagesProviderStream for ScriptedFollowUp {
    type Error = Infallible;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Infallible> {
        next_frame(&mut self.frames, &self.gate, &self.effects, self.labels).await
    }

    async fn aclose(&mut self) -> Option<Result<(), Infallible>> {
        self.effects.record("followup-close");
        Some(Ok(()))
    }
}

impl AgenticFollowUp for ScriptedFollowUp {
    fn from_response(response: &Json) -> Result<Self, MessagesFakeStreamError> {
        Ok(Self {
            frames: MessagesFakeStream::new(response)?.collect(),
            gate: None,
            effects: EffectLog::default(),
            labels: ("followup-read", "followup-end"),
        })
    }
}

struct ScriptedInner {
    frames: VecDeque<Vec<u8>>,
    gate: Option<Arc<AtomicBool>>,
    effects: EffectLog,
    labels: (&'static str, &'static str),
}

impl ScriptedInner {
    fn open(frames: Vec<Vec<u8>>, effects: &EffectLog) -> Self {
        Self {
            frames: VecDeque::from(frames),
            gate: None,
            effects: effects.clone(),
            labels: ("inner-read", "inner-end"),
        }
    }
}

#[async_trait]
impl MessagesProviderStream for ScriptedInner {
    type Error = Infallible;

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Infallible> {
        next_frame(&mut self.frames, &self.gate, &self.effects, self.labels).await
    }

    async fn aclose(&mut self) -> Option<Result<(), Infallible>> {
        self.effects.record("inner-close");
        Some(Ok(()))
    }
}

fn agentic_input(hold_back: Option<HoldBackPolicy>) -> MessagesAgenticStreamInput {
    MessagesAgenticStreamInput {
        model: "claude-test".to_owned(),
        messages: Vec::new(),
        optional_request_params: IndexMap::new(),
        logging: AgenticLogging {
            call_id: Some("call-1".to_owned()),
            agentic_loop_params: None,
        },
        custom_llm_provider: "anthropic".to_owned(),
        kwargs: IndexMap::new(),
        hold_back,
    }
}

fn policy_at(names: &[&str], ping_interval_seconds: f64) -> HoldBackPolicy {
    HoldBackPolicy {
        server_fulfilled_tool_names: names.iter().map(|name| (*name).to_owned()).collect(),
        ping_interval_seconds,
    }
}

fn policy(names: &[&str]) -> HoldBackPolicy {
    policy_at(names, PING_INTERVAL_SECONDS)
}

async fn drain<Inner>(
    stream: &mut MessagesAgenticStream<Inner, ScriptedInterceptor>,
) -> Vec<Vec<u8>>
where
    Inner: MessagesProviderStream<Error = Infallible> + Send,
{
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next_chunk().await.expect("the stream yields") {
        chunks.push(chunk);
        assert!(chunks.len() <= MAX_DRAINED, "the stream ends");
    }
    chunks
}

fn buffered(stream: &MessagesAgenticStream<ScriptedInner, ScriptedInterceptor>) -> bool {
    stream.buffered_provider_output() == Some(Json::Bool(true))
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

fn field(value: &Json, key: &str) -> Json {
    match value {
        Json::Object(fields) => fields.get(key).cloned().unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Json::Null,
    }
}

fn block_at(response: &Json, index: usize) -> Json {
    match field(response, "content") {
        Json::List(blocks) => blocks[index].clone(),
        _ => panic!("the response lists content blocks"),
    }
}

fn contains_text(chunks: &[Vec<u8>], needle: &str) -> bool {
    chunks
        .iter()
        .any(|chunk| from_utf8(chunk).is_ok_and(|text| text.contains(needle)))
}

fn starts_with_event(chunk: &[u8], event: &str) -> bool {
    from_utf8(chunk).is_ok_and(|text| text.starts_with(&format!("event: {event}\n")))
}

fn seen_tool_use(seen: &HookSeen, tool_name: &str) -> bool {
    let entries = seen.entries();
    if entries.len() != 1 {
        return false;
    }
    let block = block_at(&entries[0].0, 0);
    field(&block, "type") == Json::Text("tool_use".to_owned())
        && field(&block, "name") == Json::Text(tool_name.to_owned())
        && field(&block, "input") == json(r#"{"query":"rust"}"#)
}

fn frame(event: &str, data: &str) -> Vec<u8> {
    format!("event: {event}\ndata: {data}\n\n").into_bytes()
}

fn message_start() -> Vec<u8> {
    frame(
        "message_start",
        r#"{"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-test","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":11}}}"#,
    )
}

fn text_frames(id: &str, text: &str, input_tokens: u32, output_tokens: u32) -> Vec<Vec<u8>> {
    let start = format!(
        r#"{{"type":"message_start","message":{{"id":"{id}","type":"message","role":"assistant","model":"claude-test","content":[],"stop_reason":null,"stop_sequence":null,"usage":{{"input_tokens":{input_tokens}}}}}}}"#
    );
    let delta = format!(
        r#"{{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{text}"}}}}"#
    );
    let stop = format!(
        r#"{{"type":"message_delta","delta":{{"stop_reason":"end_turn"}},"usage":{{"output_tokens":{output_tokens}}}}}"#
    );
    vec![
        frame("message_start", &start),
        frame(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        ),
        frame("content_block_delta", &delta),
        frame(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        ),
        frame("message_delta", &stop),
        frame("message_stop", r#"{"type":"message_stop"}"#),
    ]
}

fn text_stream() -> Vec<Vec<u8>> {
    text_frames("msg_1", "Hello", 11, 7)
}

fn server_tool_stream(tool_name: &str) -> Vec<Vec<u8>> {
    let start = format!(
        r#"{{"type":"content_block_start","index":0,"content_block":{{"type":"tool_use","id":"toolu_1","name":"{tool_name}","input":{{}}}}}}"#
    );
    vec![
        message_start(),
        frame("content_block_start", &start),
        frame(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":"}}"#,
        ),
        frame(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"rust\"}"}}"#,
        ),
        frame(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        ),
        frame(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":9}}"#,
        ),
        frame("message_stop", r#"{"type":"message_stop"}"#),
    ]
}

fn replacement() -> Json {
    json(
        r#"{"id":"msg_replaced","type":"message","role":"assistant","model":"claude-test","content":[{"type":"text","text":"replaced"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":4,"output_tokens":2}}"#,
    )
}

fn replacement_frames() -> Vec<Vec<u8>> {
    text_frames("msg_replaced", "replaced", 4, 2)
}

#[derive(Clone, Copy)]
enum ReplyKind {
    Frames,
    Message,
}

fn follow_up_reply(kind: ReplyKind) -> HookReply {
    match kind {
        ReplyKind::Frames => HookReply::Stream(replacement_frames(), None),
        ReplyKind::Message => HookReply::Message(replacement()),
    }
}

// the held-back tool call is replaced before any byte reaches the client, and the interceptor sees it first
#[rstest]
#[case::streamed_replacement(ReplyKind::Frames)]
#[case::message_replacement(ReplyKind::Message)]
#[tokio::test]
async fn server_tool_held_back(#[case] kind: ReplyKind) {
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(server_tool_stream("web_search"), &effects),
        resolved_interceptor(follow_up_reply(kind), &effects, &seen),
        agentic_input(Some(policy(&["web_search"]))),
        immediate_sleep(),
        ApiLimits::default(),
    );
    let first = stream.next_chunk().await.expect("the held-back read");
    assert_eq!(first, Some(KEEPALIVE_PING.to_vec()));
    assert!(!buffered(&stream));
    assert_eq!(effects.count("inner-read"), 0);
    let chunks = drain(&mut stream).await;
    assert!(seen_tool_use(&seen, "web_search"));
    assert_eq!(effects.count("hook-start"), 1);
    // the replacement may share framing bytes with the held stream, but never the held tool call
    assert!(!contains_text(&chunks, "toolu_1"));
    if matches!(kind, ReplyKind::Frames) {
        assert_eq!(chunks, replacement_frames());
    } else {
        assert!(contains_text(&chunks, "replaced"));
        assert!(
            chunks
                .first()
                .is_some_and(|chunk| starts_with_event(chunk, "message_start"))
        );
        assert!(
            chunks
                .last()
                .is_some_and(|chunk| starts_with_event(chunk, "message_stop"))
        );
    }
}

// while the interceptor runs the client gets keepalive pings at the configured interval, then the buffer replays
#[rstest]
#[case::default_interval(15.0_f64)]
#[case::custom_interval(0.05_f64)]
#[tokio::test]
async fn ping_during_interceptor(#[case] interval: f64) {
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let gate = Arc::new(AtomicBool::new(false));
    let delays = DelayLog::default();
    let frames = text_stream();
    let interceptor = ScriptedInterceptor {
        gate: Arc::clone(&gate),
        reply: Mutex::new(Some(HookReply::Empty)),
        effects: effects.clone(),
        seen: seen.clone(),
    };
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(frames.clone(), &effects),
        interceptor,
        agentic_input(Some(policy_at(&[], interval))),
        recorded_sleep(&delays),
        ApiLimits::default(),
    );
    assert_eq!(
        stream.next_chunk().await.expect("the first held-back read"),
        Some(KEEPALIVE_PING.to_vec())
    );
    assert!(!buffered(&stream));
    assert_eq!(
        stream.next_chunk().await.expect("the draining read"),
        Some(KEEPALIVE_PING.to_vec())
    );
    assert!(buffered(&stream));
    assert_eq!(
        stream.next_chunk().await.expect("the intercepted read"),
        Some(KEEPALIVE_PING.to_vec())
    );
    let recorded = delays.snapshot();
    assert!(!recorded.is_empty());
    assert!(recorded.iter().all(|bits| *bits == interval.to_bits()));
    gate.store(true, Ordering::SeqCst);
    let chunks = drain(&mut stream).await;
    assert_eq!(chunks, frames);
    assert_eq!(effects.count("hook-start"), 1);
    assert_eq!(effects.count("inner-read"), frames.len());
    assert_eq!(effects.count("inner-end"), 1);
}

// a follow-up returned at exhaustion continues the same stream, after the live bytes
#[rstest]
#[case::streamed_follow_up(ReplyKind::Frames)]
#[case::message_follow_up(ReplyKind::Message)]
#[tokio::test]
async fn followup_stream(#[case] kind: ReplyKind) {
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let frames = text_stream();
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(frames.clone(), &effects),
        resolved_interceptor(follow_up_reply(kind), &effects, &seen),
        agentic_input(None),
        immediate_sleep(),
        ApiLimits::default(),
    );
    let first = stream.next_chunk().await.expect("the live read");
    assert_eq!(first, Some(frames[0].clone()));
    assert!(!buffered(&stream));
    let chunks = drain(&mut stream).await;
    let (live, follow_up) = chunks.split_at(frames[1..].len());
    assert_eq!(live, &frames[1..]);
    if matches!(kind, ReplyKind::Frames) {
        assert_eq!(follow_up, replacement_frames().as_slice());
    } else {
        assert!(contains_text(follow_up, "replaced"));
        assert!(starts_with_event(&follow_up[0], "message_start"));
        assert!(
            follow_up
                .last()
                .is_some_and(|chunk| starts_with_event(chunk, "message_stop"))
        );
    }
    assert!(!chunks.contains(&KEEPALIVE_PING.to_vec()));
    assert_eq!(effects.count("hook-start"), 1);
    assert_eq!(seen.entries()[0].1, "claude-test");
}

const LEAK_LOG: &str = "ERROR AgenticStreamingIterator: hooks did not replace a message containing a server-fulfilled tool_use [model=claude-test]; emitting an SSE error instead of leaking the tool call to the client";

// an unreplaced server-fulfilled tool call becomes the SSE error frame once and ends the turn
#[rstest]
#[case::exact_name("web_search", true)]
#[case::case_mismatch("Web_Search", false)]
#[case::name_prefix("web", false)]
#[tokio::test]
async fn leaked_server_tool_error(#[case] tool_name: &str, #[case] leaks: bool) {
    let (logs, _guard) = capture_logs();
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let frames = server_tool_stream(tool_name);
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(frames.clone(), &effects),
        resolved_interceptor(HookReply::Empty, &effects, &seen),
        agentic_input(Some(policy(&["web_search"]))),
        immediate_sleep(),
        ApiLimits::default(),
    );
    assert_eq!(
        stream.next_chunk().await.expect("the held-back read"),
        Some(KEEPALIVE_PING.to_vec())
    );
    let chunks = drain(&mut stream).await;
    assert!(seen_tool_use(&seen, tool_name));
    assert_eq!(effects.count("hook-start"), 1);
    assert_eq!(
        chunks
            .iter()
            .any(|chunk| is_server_fulfilled_tool_leak_error(chunk)),
        leaks
    );
    assert_eq!(logs.count(LEAK_LOG), usize::from(leaks));
    if leaks {
        assert_eq!(chunks.len(), 1);
        assert!(is_server_fulfilled_tool_leak_error(&chunks[0]));
        assert!(chunks.iter().all(|chunk| !frames.contains(chunk)));
    } else {
        assert_eq!(chunks, frames);
    }
}

// aclose settles whatever the iterator owns, then closes the wrapped stream
#[rstest]
#[case::while_draining(false)]
#[case::while_intercepting(true)]
#[tokio::test]
async fn cancel_drains_owned_task(#[case] intercepting: bool) {
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let hook_gate = Arc::new(AtomicBool::new(false));
    let inner = ScriptedInner {
        frames: VecDeque::from(text_stream()),
        gate: Some(Arc::new(AtomicBool::new(intercepting))),
        effects: effects.clone(),
        labels: ("inner-read", "inner-end"),
    };
    let interceptor = ScriptedInterceptor {
        gate: Arc::clone(&hook_gate),
        reply: Mutex::new(Some(HookReply::Empty)),
        effects: effects.clone(),
        seen: seen.clone(),
    };
    let mut stream = MessagesAgenticStream::new(
        inner,
        interceptor,
        agentic_input(Some(policy(&[]))),
        immediate_sleep(),
        ApiLimits::default(),
    );
    assert_eq!(
        stream.next_chunk().await.expect("the first held-back read"),
        Some(KEEPALIVE_PING.to_vec())
    );
    assert_eq!(
        stream.next_chunk().await.expect("the held read"),
        Some(KEEPALIVE_PING.to_vec())
    );
    let closed = stream.aclose().await.expect("the iterator supports aclose");
    closed.expect("the close succeeds");
    assert_eq!(
        effects.snapshot().last().map(String::as_str),
        Some("inner-close")
    );
    if intercepting {
        assert_eq!(effects.count("hook-start"), 1);
        assert_eq!(effects.count("hook-cancelled"), 1);
    } else {
        assert_eq!(effects.count("hook-start"), 0);
        assert_eq!(effects.count("hook-cancelled"), 0);
    }
    assert_eq!(stream.next_chunk().await.expect("the closed read"), None);
}

#[derive(Clone, Copy)]
enum Fallback {
    Unexpected,
    Failed,
    Unbuildable,
}

fn fallback_reply(fallback: Fallback) -> HookReply {
    match fallback {
        Fallback::Unexpected => HookReply::Unexpected("dict"),
        Fallback::Failed => HookReply::Failed("hook exploded".to_owned()),
        Fallback::Unbuildable => HookReply::Message(json(r#"{"content":null}"#)),
    }
}

// a hook answer no iterator can serve is logged and skipped, leaving the live bytes alone
#[rstest]
#[case::unexpected_result(
    Fallback::Unexpected,
    Some("call-1"),
    "WARN AgenticStreamingIterator: Unexpected result type from hooks: dict"
)]
#[case::hook_error(
    Fallback::Failed,
    Some("call-1"),
    "ERROR AgenticStreamingIterator: Error in agentic hook processing [call_id=call-1 model=claude-test]: callback scripted-hook failed: hook exploded"
)]
#[case::hook_error_without_call_id(
    Fallback::Failed,
    None,
    "ERROR AgenticStreamingIterator: Error in agentic hook processing [call_id=unknown model=claude-test]: callback scripted-hook failed: hook exploded"
)]
#[case::unbuildable_message(
    Fallback::Unbuildable,
    Some("call-1"),
    "ERROR AgenticStreamingIterator: Error in agentic hook processing [call_id=call-1 model=claude-test]: 'NoneType' object is not iterable"
)]
#[tokio::test]
async fn hook_outcome_fallbacks(
    #[case] fallback: Fallback,
    #[case] call_id: Option<&str>,
    #[case] logged: &str,
) {
    let (logs, _guard) = capture_logs();
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let frames = text_stream();
    let mut input = agentic_input(None);
    input.logging.call_id = call_id.map(str::to_owned);
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(frames.clone(), &effects),
        resolved_interceptor(fallback_reply(fallback), &effects, &seen),
        input,
        immediate_sleep(),
        ApiLimits::default(),
    );
    let chunks = drain(&mut stream).await;
    assert_eq!(chunks, frames);
    assert_eq!(effects.count("hook-start"), 1);
    assert_eq!(seen.entries().len(), 1);
    assert_eq!(logs.count(logged), 1);
}

#[derive(Clone, Copy)]
enum BufferShape {
    NoMessageStart,
    WrongShape,
}

fn unrebuildable(shape: BufferShape) -> Vec<Vec<u8>> {
    match shape {
        BufferShape::NoMessageStart => vec![
            frame(
                "content_block_start",
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            ),
            frame("message_stop", r#"{"type":"message_stop"}"#),
        ],
        BufferShape::WrongShape => vec![frame("message_start", "[]")],
    }
}

// bytes that rebuild to nothing skip the hooks instead of failing the turn
#[rstest]
#[case::no_message_start(
    BufferShape::NoMessageStart,
    "DEBUG AgenticStreamingIterator: Could not rebuild response from SSE bytes"
)]
#[case::wrong_shape(
    BufferShape::WrongShape,
    "ERROR AgenticStreamingIterator: Error in agentic hook processing [call_id=call-1 model=claude-test]: the data field of an agentic stream event is not an object"
)]
#[tokio::test]
async fn unrebuildable_buffer_skips_hooks(#[case] shape: BufferShape, #[case] logged: &str) {
    let (logs, _guard) = capture_logs();
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let frames = unrebuildable(shape);
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(frames.clone(), &effects),
        resolved_interceptor(HookReply::Empty, &effects, &seen),
        agentic_input(None),
        immediate_sleep(),
        ApiLimits::default(),
    );
    let chunks = drain(&mut stream).await;
    assert_eq!(chunks, frames);
    assert_eq!(effects.count("hook-start"), 0);
    assert!(seen.entries().is_empty());
    assert_eq!(logs.count(logged), 1);
}

// an exhausted stream that collected nothing never calls the interceptor
#[rstest]
#[case::live(false)]
#[case::held_back(true)]
#[tokio::test]
async fn empty_buffer_skips_hooks(#[case] held_back: bool) {
    let effects = EffectLog::default();
    let seen = HookSeen::default();
    let mut stream = MessagesAgenticStream::new(
        ScriptedInner::open(Vec::new(), &effects),
        resolved_interceptor(HookReply::Empty, &effects, &seen),
        agentic_input(held_back.then_some(policy(&[]))),
        immediate_sleep(),
        ApiLimits::default(),
    );
    let chunks = drain(&mut stream).await;
    if held_back {
        assert_eq!(chunks, vec![KEEPALIVE_PING.to_vec()]);
    } else {
        assert!(chunks.is_empty());
    }
    assert_eq!(effects.count("hook-start"), 0);
}

// a whole-message hook answer replays as the same frames the fake stream builds
#[tokio::test]
async fn fake_follow_up_replays_message() {
    let expected = MessagesFakeStream::new(&replacement())
        .expect("the replacement builds frames")
        .collect::<Vec<_>>();
    let mut follow_up =
        MessagesFakeFollowUp::from_response(&replacement()).expect("the follow-up builds");
    let mut chunks = Vec::new();
    while let Some(chunk) = follow_up.next_chunk().await.expect("the follow-up yields") {
        chunks.push(chunk);
        assert!(chunks.len() <= MAX_DRAINED, "the follow-up ends");
    }
    assert!(!expected.is_empty());
    assert_eq!(chunks, expected);
}

#[test]
fn replays_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/messages_agentic_stream");
    let seeds: Vec<Vec<u8>> = fs::read_dir(corpus)
        .expect("the corpus folder")
        .map(|entry| fs::read(entry.expect("an entry").path()).expect("a seed file"))
        .collect();
    assert!(!seeds.is_empty());
    let decoded = seeds
        .iter()
        .filter(|bytes| {
            decode_messages_agentic_stream(bytes.as_slice(), &ApiLimits::default()).is_ok()
        })
        .count();
    assert!(decoded >= 1);
}
