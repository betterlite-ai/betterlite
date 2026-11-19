use std::collections::VecDeque;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_prompts::arguments::json::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use chrono::DateTime;
use indexmap::IndexMap;
use serde_json::json;
use uuid::Uuid;

use crate::api::messages_chat_stream::sse::SyncFrames;
use crate::api::messages_chat_stream::view::AnthropicSseStream;
use crate::api::messages_chat_transform::stream::{StreamOptions, StreamOutput};
use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, CompactionKind, PolyfillResult, UsageIteration, UsageKind,
};
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::messages_stream_split::{AsyncChunkSource, ChunkSource};

use super::adapter;

type AsyncView = AnthropicSseStream<Scripted, FixedIds>;
type SyncIterator = SyncFrames<Scripted, FixedIds>;

#[derive(Clone)]
struct Scripted {
    items: VecDeque<StreamChunk>,
    chunks: Vec<StreamChunk>,
}

impl ChunkSource for Scripted {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        let Some(chunk) = self.items.pop_front() else {
            return Ok(None);
        };
        self.chunks.push(chunk.clone());
        Ok(Some(chunk))
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        Some(&self.chunks)
    }
}

#[async_trait]
impl AsyncChunkSource for Scripted {
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        let Some(chunk) = self.items.pop_front() else {
            return Ok(None);
        };
        self.chunks.push(chunk.clone());
        Ok(Some(chunk))
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        Some(&self.chunks)
    }
}

struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(7)
    }
}

fn chunk(source: &str) -> StreamChunk {
    serde_json::from_str(source).expect("a valid chunk fixture")
}

fn text(value: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[{{"index":0,"delta":{{"content":"{value}"}},"finish_reason":null}}]}}"#
    ))
}

fn finish(reason: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[{{"index":0,"delta":{{}},"finish_reason":"{reason}"}}]}}"#
    ))
}

fn usage_chunk(prompt: i64, completion: i64) -> StreamChunk {
    chunk(&format!(
        r#"{{"choices":[],"usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion}}}}}"#
    ))
}

fn tool_call(name: &str) -> StreamChunk {
    let call = json!({
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": name, "arguments": "{\"x\":1}"}
                }]
            },
            "finish_reason": null
        }]
    });
    serde_json::from_value(call).expect("a valid tool-call chunk")
}

fn scripted(chunks: Vec<StreamChunk>) -> Scripted {
    Scripted {
        items: chunks.into_iter().collect(),
        chunks: Vec::new(),
    }
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("a valid JSON fixture")
}

// The frames as (event name, data), with the generated message id replaced by one value.
fn frames(frames: &[String], message_id: &str) -> Vec<(String, Json)> {
    frames
        .iter()
        .map(|frame| {
            let normalized = frame.replace(message_id, "msg_fixed");
            let mut lines = normalized.lines();
            let event = lines
                .next()
                .expect("an event line")
                .strip_prefix("event: ")
                .expect("the event prefix")
                .to_owned();
            let data = lines
                .next()
                .expect("a data line")
                .strip_prefix("data: ")
                .expect("the data prefix");
            (event, json(data))
        })
        .collect()
}

// The value under a path inside one event's data object; a test names a path it knows exists.
fn at<'events>(events: &'events [(String, Json)], index: usize, path: &[&str]) -> &'events Json {
    let mut current: &Json = &events[index].1;
    for key in path {
        current = current
            .as_object()
            .and_then(|fields| fields.get(*key))
            .expect("the event carries the asserted path");
    }
    current
}

async fn drain_async(view: &mut AsyncView) -> (Vec<String>, String) {
    let message_id = view.stream.state.message_id.clone();
    let mut collected = Vec::new();
    while let Some(frame) = view.next_frame().await {
        collected.push(String::from_utf8(frame).expect("the frame is text"));
    }
    (collected, message_id)
}

fn drain_sync(driver: SyncIterator) -> (Vec<String>, String) {
    let message_id = driver.driver.state.message_id.clone();
    let collected = driver
        .map(|frame| {
            String::from_utf8(frame.expect("the frame encodes")).expect("the frame is text")
        })
        .collect();
    (collected, message_id)
}

fn event_names(events: &[(String, Json)]) -> Vec<&str> {
    events.iter().map(|(name, _)| name.as_str()).collect()
}

fn text_stream() -> Scripted {
    scripted(vec![text("hello"), finish("stop"), usage_chunk(10, 2)])
}

#[tokio::test]
async fn stream_factory_defaults_to_the_async_view() {
    let output = adapter().translate_completion_output_params_streaming(
        text_stream(),
        StreamOptions::new("gpt-4o"),
        FixedIds,
    );
    let StreamOutput::Async(mut view) = output else {
        panic!("is_async defaults to true");
    };
    let (collected, message_id) = drain_async(&mut view).await;
    let events = frames(&collected, &message_id);
    assert_eq!(
        event_names(&events),
        [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ]
    );
    assert_eq!(at(&events, 0, &["message", "model"]), &json(r#""gpt-4o""#));
    assert_eq!(at(&events, 0, &["message", "id"]), &json(r#""msg_fixed""#));
    assert_eq!(
        at(&events, 2, &["delta"]),
        &json(r#"{"type": "text_delta", "text": "hello"}"#)
    );
    assert_eq!(
        at(&events, 4, &["delta", "stop_reason"]),
        &json(r#""end_turn""#)
    );
    assert_eq!(at(&events, 4, &["usage", "input_tokens"]), &json("10"));
    assert_eq!(at(&events, 4, &["usage", "output_tokens"]), &json("2"));

    assert_eq!(view.model(), "gpt-4o");
    assert_eq!(
        view.chunks().map(<[StreamChunk]>::len),
        Some(3),
        "every provider chunk is recorded for partial billing"
    );
    assert!(
        view.messages().is_none(),
        "a scripted source exposes no chat messages"
    );
    view.aclose();
    assert!(
        view.next_frame().await.is_none(),
        "a closed view yields nothing"
    );
}

#[tokio::test]
async fn stream_factory_returns_a_sync_iterator_when_asked() {
    let mut options = StreamOptions::new("gpt-4o");
    options.is_async = false;
    let output =
        adapter().translate_completion_output_params_streaming(text_stream(), options, FixedIds);
    let StreamOutput::Sync(driver) = output else {
        panic!("is_async=false returns the sync iterator");
    };
    let (collected, message_id) = drain_sync(*driver);

    let async_output = adapter().translate_completion_output_params_streaming(
        text_stream(),
        StreamOptions::new("gpt-4o"),
        FixedIds,
    );
    let StreamOutput::Async(mut view) = async_output else {
        panic!("is_async defaults to true");
    };
    let (async_collected, async_id) = drain_async(&mut view).await;
    assert_eq!(
        frames(&collected, &message_id),
        frames(&async_collected, &async_id),
        "both modes emit the same events"
    );
}

#[tokio::test]
async fn stream_factory_restores_tool_names_through_the_mapping() {
    let long = "a".repeat(65);
    let short = format!("{}_635361c4", "a".repeat(55));
    let mapping = IndexMap::from([(short.clone(), long.clone())]);
    let mut options = StreamOptions::new("gpt-4o");
    options.tool_name_mapping = Some(&mapping);
    let output = adapter().translate_completion_output_params_streaming(
        scripted(vec![tool_call(&short), finish("tool_calls")]),
        options,
        FixedIds,
    );
    let StreamOutput::Async(mut view) = output else {
        panic!("is_async defaults to true");
    };
    let (collected, message_id) = drain_async(&mut view).await;
    let events = frames(&collected, &message_id);
    assert_eq!(
        event_names(&events),
        [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ]
    );
    assert_eq!(
        at(&events, 1, &["content_block", "type"]),
        &json(r#""tool_use""#)
    );
    assert_eq!(
        at(&events, 1, &["content_block", "name"]),
        &json(&format!("\"{long}\""))
    );
    assert_eq!(
        at(&events, 4, &["delta", "stop_reason"]),
        &json(r#""tool_use""#)
    );
}

fn polyfill() -> PolyfillResult {
    PolyfillResult {
        messages: Vec::new(),
        system: None,
        applied_edits: vec![AppliedEdit {
            edit_type: Some("clear_tool_uses_20250919".to_owned()),
            cleared_tool_uses: Some(1),
            ..AppliedEdit::default()
        }],
        compaction_block: Some(CompactionBlock {
            kind: CompactionKind::Compaction,
            content: Some("sum".to_owned()),
            signature: Some("s".to_owned()),
        }),
        iterations_usage: Some(vec![UsageIteration {
            kind: UsageKind::Compaction,
            input_tokens: Some(4),
            output_tokens: Some(1),
        }]),
    }
}

fn polyfilled_events(events: &[(String, Json)]) {
    assert_eq!(
        at(events, 1, &["content_block", "type"]),
        &json(r#""compaction""#),
        "the compaction block opens before the text"
    );
    assert_eq!(
        at(events, 7, &["context_management", "applied_edits"]),
        &json(r#"[{"type": "clear_tool_uses_20250919", "cleared_tool_uses": 1}]"#)
    );
    assert_eq!(
        at(events, 7, &["usage", "iterations"]),
        &json(
            r#"[{"type": "compaction", "input_tokens": 4, "output_tokens": 1},
                {"type": "message", "input_tokens": 10, "output_tokens": 2}]"#
        ),
        "the summary iteration comes before the message iteration"
    );
}

#[tokio::test]
async fn stream_factory_passes_the_polyfill_output_to_both_modes() {
    let polyfill = polyfill();
    let mut options = StreamOptions::new("gpt-4o");
    options.polyfill_result = Some(&polyfill);
    let output =
        adapter().translate_completion_output_params_streaming(text_stream(), options, FixedIds);
    let StreamOutput::Async(mut view) = output else {
        panic!("is_async defaults to true");
    };
    let (collected, message_id) = drain_async(&mut view).await;
    let events = frames(&collected, &message_id);
    assert_eq!(
        event_names(&events),
        [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop"
        ]
    );
    polyfilled_events(&events);

    let mut options = StreamOptions::new("gpt-4o");
    options.polyfill_result = Some(&polyfill);
    options.is_async = false;
    let output =
        adapter().translate_completion_output_params_streaming(text_stream(), options, FixedIds);
    let StreamOutput::Sync(driver) = output else {
        panic!("is_async=false returns the sync iterator");
    };
    let (collected, message_id) = drain_sync(*driver);
    polyfilled_events(&frames(&collected, &message_id));
}

#[tokio::test]
async fn stream_factory_records_the_message_id_on_the_callers_logging_state() {
    let mut logging = logging_state();
    let output = {
        let mut options = StreamOptions::new("gpt-4o");
        options.logging = Some(&mut logging);
        adapter().translate_completion_output_params_streaming(text_stream(), options, FixedIds)
    };
    let StreamOutput::Async(mut view) = output else {
        panic!("is_async defaults to true");
    };
    let message_id = view.stream.state.message_id.clone();
    assert_eq!(
        logging.streamed_anthropic_message_id.as_deref(),
        Some(message_id.as_str()),
        "the caller's logging state received the stream's message id"
    );
    let (collected, _) = drain_async(&mut view).await;
    assert_eq!(
        at(&frames(&collected, &message_id), 0, &["message", "id"]),
        &json(r#""msg_fixed""#)
    );
}

fn logging_state() -> LoggingState {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "claude".to_owned(),
            call_type: "anthropic_messages".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .expect("the fixture time is RFC 3339"),
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor compiles"),
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}
