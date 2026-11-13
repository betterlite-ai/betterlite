#![cfg(test)]

use std::collections::VecDeque;

use async_trait::async_trait;
use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{
    JsonValue, StreamChoice, StreamChunk, StreamDelta,
};
use tokio::runtime::{Builder, Runtime};

use super::delta::StreamingDeltaType;
use super::errors::{
    INCOMPLETE_STREAM_MESSAGE, StreamFailure, error_status_and_message, mid_stream_error_sse_event,
};
use super::split::split_chunk;
use super::{AsyncChunkSource, AsyncChunkSplitter, ChunkSource, ChunkSplitter, next_split_chunk};

const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0001_0000,
    max_event_bytes: 0x0001_0000,
    max_items: 1_000,
};

const USAGE: &str = r#""usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}"#;

fn chunk(source: &str) -> StreamChunk {
    serde_json::from_str(source).expect("the chunk fixture deserializes")
}

fn text_chunk(text: &str, finish_reason: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"id":"chunk-1","object":"chat.completion.chunk","created":7,"model":"gpt-4o-mini","choices":[{{"index":0,"delta":{{"role":"assistant","content":"{text}"}},"finish_reason":{finish_reason}}}]}}"#
    ))
}

fn single(chunks: &[StreamChunk]) -> &StreamChunk {
    chunks.first().expect("the splitter returned a piece")
}

fn two(chunks: &[StreamChunk]) -> (&StreamChunk, &StreamChunk) {
    assert_eq!(chunks.len(), 2);
    (
        chunks.first().expect("content piece"),
        chunks.get(1).expect("finish piece"),
    )
}

fn only_choice(piece: &StreamChunk) -> &StreamChoice {
    piece.choices.first().expect("the piece carries one choice")
}

fn delta(piece: &StreamChunk) -> &StreamDelta {
    &only_choice(piece).delta
}

struct BareSource;

impl ChunkSource for BareSource {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        Ok(None)
    }
}

#[async_trait]
impl AsyncChunkSource for BareSource {
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        Ok(None)
    }
}

struct TestSource {
    chunks: VecDeque<StreamChunk>,
    failure: Option<StreamFailure>,
    chunks_attr: Option<Vec<StreamChunk>>,
    messages_attr: Option<Vec<Json>>,
    pulls: usize,
}

impl TestSource {
    fn new(chunks: Vec<StreamChunk>) -> Self {
        Self {
            chunks: VecDeque::from(chunks),
            failure: None,
            chunks_attr: None,
            messages_attr: None,
            pulls: 0,
        }
    }

    fn failing(failure: StreamFailure) -> Self {
        Self {
            chunks: VecDeque::new(),
            failure: Some(failure),
            chunks_attr: None,
            messages_attr: None,
            pulls: 0,
        }
    }

    fn pull(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pulls = self.pulls.saturating_add(1);
        if let Some(chunk) = self.chunks.pop_front() {
            return Ok(Some(chunk));
        }
        self.failure.clone().map_or(Ok(None), Err)
    }
}

impl ChunkSource for TestSource {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pull()
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        self.chunks_attr.as_deref()
    }

    fn messages(&self) -> Option<&[Json]> {
        self.messages_attr.as_deref()
    }
}

#[async_trait]
impl AsyncChunkSource for TestSource {
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pull()
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        self.chunks_attr.as_deref()
    }

    fn messages(&self) -> Option<&[Json]> {
        self.messages_attr.as_deref()
    }
}

fn splitter(source: TestSource) -> ChunkSplitter<TestSource> {
    ChunkSplitter {
        source,
        buffer: VecDeque::new(),
    }
}

fn async_splitter(source: TestSource) -> AsyncChunkSplitter<TestSource> {
    AsyncChunkSplitter {
        source,
        buffer: VecDeque::new(),
    }
}

fn current_thread() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("a current-thread runtime")
}

// finish_and_content_split

#[test]
fn finish_and_content_split_peels_content_then_finish() {
    let combined = chunk(&format!(
        r#"{{"id":"chunk-1","created":7,"model":"m","choices":[{{"index":0,"delta":{{"role":"assistant","content":"Hello"}},"finish_reason":"stop"}}],{USAGE},"hidden_params":{{"usage":{{"prompt_tokens":5}},"request_id":"req-1"}}}}"#
    ));
    let pieces = split_chunk(&combined);
    let (content, finish) = two(&pieces);
    assert_eq!(only_choice(content).finish_reason, None);
    assert_eq!(delta(content).content.as_deref(), Some("Hello"));
    assert_eq!(content.usage, None);
    assert!(!content.hidden_params.contains_key("usage"));
    assert_eq!(
        content.hidden_params.get("request_id"),
        Some(&JsonValue::String("req-1".to_owned()))
    );
    assert_eq!(only_choice(finish).finish_reason.as_deref(), Some("stop"));
    assert_eq!(delta(finish).content, None);
    assert_eq!(delta(finish).reasoning_content, None);
    assert_eq!(delta(finish).thinking_blocks, None);
    assert_eq!(delta(finish).tool_calls, None);
    assert_eq!(
        finish.usage.as_ref().map(|usage| usage.prompt_tokens),
        Some(5)
    );
    assert!(finish.hidden_params.contains_key("usage"));
}

#[test]
fn finish_and_content_split_keeps_one_chunk_without_payload_or_finish() {
    let without_finish = text_chunk("Hello", "null");
    assert_eq!(split_chunk(&without_finish).len(), 1);
    let without_payload = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":"stop"}]}"#,
    );
    assert_eq!(split_chunk(&without_payload).len(), 1);
}

#[test]
fn finish_and_content_split_keeps_refusal_on_the_finish_copy() {
    let refusal = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"role":"assistant","refusal":"no"},"finish_reason":"stop"}]}"#,
    );
    let pieces = split_chunk(&refusal);
    let (content, finish) = two(&pieces);
    assert_eq!(
        delta(content).extra.get("refusal"),
        Some(&JsonValue::String("no".to_owned()))
    );
    assert_eq!(
        delta(finish).extra.get("refusal"),
        Some(&JsonValue::String("no".to_owned()))
    );
}

#[test]
fn finish_and_content_split_reads_refusal_from_provider_fields() {
    let refusal = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"role":"assistant","provider_specific_fields":{"refusal":"no"}},"finish_reason":"stop"}]}"#,
    );
    assert_eq!(split_chunk(&refusal).len(), 2);
}

#[test]
fn finish_and_content_split_copies_other_choices_unchanged() {
    let combined = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"first"},"finish_reason":"stop"},{"index":1,"delta":{"content":"second"},"finish_reason":"length"}]}"#,
    );
    let pieces = split_chunk(&combined);
    let (content, finish) = two(&pieces);
    assert_eq!(content.choices.len(), 2);
    assert_eq!(finish.choices.len(), 2);
    let second = finish.choices.get(1).expect("the untouched choice");
    assert_eq!(second.finish_reason.as_deref(), Some("length"));
    assert_eq!(second.delta.content.as_deref(), Some("second"));
}

// payload_order_and_roles

fn multi_payload() -> StreamChunk {
    chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"R","content":"T","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"lookup","arguments":"{}"}}],"refusal":"no","function_call":{"name":"legacy","arguments":"{}"}}}]}"#,
    )
}

#[test]
fn payload_order_and_roles_orders_reasoning_then_text_then_tools() {
    let pieces = split_chunk(&multi_payload());
    assert_eq!(pieces.len(), 3);
    let reasoning = pieces.first().expect("reasoning piece");
    let text = pieces.get(1).expect("text piece");
    let tools = pieces.get(2).expect("tools piece");
    assert_eq!(delta(reasoning).reasoning_content.as_deref(), Some("R"));
    assert_eq!(delta(reasoning).content, None);
    assert_eq!(delta(reasoning).tool_calls, None);
    assert_eq!(delta(text).content.as_deref(), Some("T"));
    assert_eq!(delta(text).reasoning_content, None);
    assert!(delta(tools).tool_calls.is_some());
    assert_eq!(delta(tools).content, None);
}

#[test]
fn payload_order_and_roles_keeps_only_the_first_role() {
    let pieces = split_chunk(&multi_payload());
    assert_eq!(
        delta(pieces.first().expect("first piece")).role.as_deref(),
        Some("assistant")
    );
    assert_eq!(delta(pieces.get(1).expect("second piece")).role, None);
    assert_eq!(delta(pieces.get(2).expect("third piece")).role, None);
}

#[test]
fn payload_order_and_roles_drops_fields_outside_each_group() {
    let pieces = split_chunk(&multi_payload());
    let reasoning = pieces.first().expect("reasoning piece");
    let text = pieces.get(1).expect("text piece");
    let tools = pieces.get(2).expect("tools piece");
    assert_eq!(delta(reasoning).extra.get("refusal"), None);
    assert_eq!(delta(text).extra.get("refusal"), None);
    assert_eq!(delta(tools).extra.get("refusal"), None);
    assert_eq!(delta(reasoning).function_call, None);
    assert_eq!(delta(text).function_call, None);
    assert_eq!(delta(tools).function_call, None);
}

#[test]
fn payload_order_and_roles_keeps_metadata_but_clears_usage_on_a_finished_chunk() {
    let combined = chunk(&format!(
        r#"{{"id":"chunk-1","created":7,"model":"m","system_fingerprint":"fp","choices":[{{"index":0,"delta":{{"content":"T","reasoning_content":"R"}},"finish_reason":"stop"}}],{USAGE}}}"#
    ));
    let pieces = split_chunk(&combined);
    let content = pieces.first().expect("content piece");
    let finish = pieces.last().expect("finish piece");
    assert_eq!(content.usage, None);
    assert_eq!(content.system_fingerprint.as_deref(), Some("fp"));
    assert_eq!(only_choice(finish).finish_reason.as_deref(), Some("stop"));
    assert_eq!(delta(finish).reasoning_content, None);
    assert_eq!(delta(finish).content, None);
    assert_eq!(
        finish.usage.as_ref().map(|usage| usage.total_tokens),
        Some(7)
    );
}

#[test]
fn payload_order_and_roles_keeps_usage_without_a_finish_reason() {
    let combined = chunk(&format!(
        r#"{{"id":"chunk-1","created":7,"model":"m","choices":[{{"index":0,"delta":{{"content":"T","reasoning_content":"R"}}}}],{USAGE}}}"#
    ));
    let pieces = split_chunk(&combined);
    assert_eq!(pieces.len(), 2);
    assert_eq!(
        pieces
            .first()
            .expect("reasoning piece")
            .usage
            .as_ref()
            .map(|usage| usage.completion_tokens),
        Some(2)
    );
    assert_eq!(
        pieces
            .get(1)
            .expect("text piece")
            .usage
            .as_ref()
            .map(|usage| usage.completion_tokens),
        Some(2)
    );
}

// continuation_and_multichoice_passthrough

#[test]
fn continuation_and_multichoice_passthrough_keeps_zero_and_two_choices() {
    let no_choices = chunk(&format!(
        r#"{{"id":"chunk-1","created":7,"model":"m","choices":[],{USAGE}}}"#
    ));
    assert_eq!(split_chunk(&no_choices).len(), 1);
    let two_choices = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"T","reasoning_content":"R"}},{"index":1,"delta":{"content":"U","reasoning_content":"S"}}]}"#,
    );
    assert_eq!(split_chunk(&two_choices).len(), 1);
}

#[test]
fn continuation_and_multichoice_passthrough_keeps_a_missing_delta() {
    let missing = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"finish_reason":"stop"}]}"#,
    );
    let pieces = split_chunk(&missing);
    assert_eq!(pieces.len(), 1);
    assert_eq!(delta(single(&pieces)), &StreamDelta::default());
}

#[test]
fn continuation_and_multichoice_passthrough_keeps_one_payload_group() {
    let content_only = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"T"}}]}"#,
    );
    assert_eq!(split_chunk(&content_only).len(), 1);
}

#[test]
fn continuation_and_multichoice_passthrough_keeps_unnamed_tool_calls() {
    let continuations = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"T","reasoning_content":"R","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":null,"arguments":"{\"a\""}}]}}]}"#,
    );
    assert_eq!(split_chunk(&continuations).len(), 1);
}

#[test]
fn continuation_and_multichoice_passthrough_keeps_tool_calls_without_a_function() {
    let continuations = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"T","reasoning_content":"R","tool_calls":[{"index":0}]}}]}"#,
    );
    assert_eq!(split_chunk(&continuations).len(), 1);
}

#[test]
fn continuation_and_multichoice_passthrough_splits_with_one_named_tool() {
    let named = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"T","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":null,"arguments":"{\"a\""}},{"index":1,"id":"call-2","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}"#,
    );
    let pieces = split_chunk(&named);
    assert_eq!(pieces.len(), 2);
    assert_eq!(
        delta(pieces.first().expect("text piece"))
            .content
            .as_deref(),
        Some("T")
    );
    assert!(
        delta(pieces.get(1).expect("tool piece"))
            .tool_calls
            .is_some()
    );
}

#[test]
fn continuation_and_multichoice_passthrough_treats_empty_values_as_absent() {
    let empty = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"","reasoning_content":"R","tool_calls":[]}}]}"#,
    );
    assert_eq!(split_chunk(&empty).len(), 1);
    let empty_blocks = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"thinking_blocks":[],"reasoning_content":"R"}}]}"#,
    );
    assert_eq!(split_chunk(&empty_blocks).len(), 1);
}

#[test]
fn continuation_and_multichoice_passthrough_splits_an_empty_finish_reason() {
    let mut combined = text_chunk("Hello", "null");
    combined
        .choices
        .first_mut()
        .expect("one choice")
        .finish_reason = Some(String::new());
    assert_eq!(split_chunk(&combined).len(), 2);
}

#[test]
fn continuation_and_multichoice_passthrough_keeps_an_empty_tool_call_list_with_text() {
    let pieces = split_chunk(&chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"T","reasoning_content":"R","tool_calls":[]}}]}"#,
    ));
    assert_eq!(pieces.len(), 2);
}

#[test]
fn continuation_and_multichoice_passthrough_keeps_a_non_text_tool_name() {
    let continuations = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"T","reasoning_content":"R","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":[],"arguments":"{}"}},{"index":1,"id":"call-2","type":"function","function":{"name":{},"arguments":"{}"}}]}}]}"#,
    );
    assert_eq!(split_chunk(&continuations).len(), 1);
}

// unsigned_thinking_normalization

fn thinking_chunk(thinking_blocks: &str, reasoning: &str) -> StreamChunk {
    chunk(&format!(
        r#"{{"id":"chunk-1","created":7,"model":"m","choices":[{{"index":0,"delta":{{"content":"T","reasoning_content":{reasoning},"thinking_blocks":{thinking_blocks}}}}}]}}"#
    ))
}

fn blocks(source: &str) -> JsonValue {
    serde_json::from_str(source).expect("the blocks round-trip")
}

#[test]
fn unsigned_thinking_normalization_joins_blocks_into_reasoning() {
    let thinking = r#"[{"type":"thinking","thinking":"A"},{"type":"thinking","thinking":"B"}]"#;
    let pieces = split_chunk(&thinking_chunk(thinking, "\"R\""));
    let reasoning = pieces.first().expect("reasoning piece");
    assert_eq!(delta(reasoning).reasoning_content.as_deref(), Some("AB"));
    assert_eq!(delta(reasoning).thinking_blocks, None);
    assert_eq!(
        delta(pieces.get(1).expect("text piece")).content.as_deref(),
        Some("T")
    );
}

#[test]
fn unsigned_thinking_normalization_keeps_signed_blocks() {
    let thinking = r#"[{"type":"thinking","thinking":"A"},{"type":"thinking","thinking":"B","signature":"sig"}]"#;
    let pieces = split_chunk(&thinking_chunk(thinking, "null"));
    let reasoning = pieces.first().expect("reasoning piece");
    assert_eq!(delta(reasoning).reasoning_content, None);
    assert_eq!(delta(reasoning).thinking_blocks, Some(blocks(thinking)));
}

#[test]
fn unsigned_thinking_normalization_keeps_a_signed_non_thinking_block() {
    let thinking = r#"[{"type":"redacted_thinking","data":"cipher","signature":"sig"},{"type":"thinking","thinking":"A"}]"#;
    let pieces = split_chunk(&thinking_chunk(thinking, "\"R\""));
    let reasoning = pieces.first().expect("reasoning piece");
    assert_eq!(delta(reasoning).reasoning_content.as_deref(), Some("R"));
    assert_eq!(delta(reasoning).thinking_blocks, Some(blocks(thinking)));
}

#[test]
fn unsigned_thinking_normalization_keeps_fields_without_text() {
    let thinking =
        r#"[{"type":"redacted_thinking","data":"cipher"},{"type":"thinking","thinking":""}]"#;
    let pieces = split_chunk(&thinking_chunk(thinking, "null"));
    let reasoning = pieces.first().expect("reasoning piece");
    assert_eq!(delta(reasoning).reasoning_content, None);
    assert_eq!(delta(reasoning).thinking_blocks, Some(blocks(thinking)));
}

#[test]
fn unsigned_thinking_normalization_joins_only_thinking_blocks() {
    let thinking =
        r#"[{"type":"redacted_thinking","thinking":"X"},{"type":"thinking","thinking":"A"}]"#;
    let pieces = split_chunk(&thinking_chunk(thinking, "null"));
    assert_eq!(
        delta(pieces.first().expect("reasoning piece"))
            .reasoning_content
            .as_deref(),
        Some("A")
    );
}

#[test]
fn unsigned_thinking_normalization_ignores_non_mapping_blocks() {
    let thinking = r#"[null,{"type":"thinking","thinking":"A"}]"#;
    let pieces = split_chunk(&thinking_chunk(thinking, "null"));
    assert_eq!(
        delta(pieces.first().expect("reasoning piece"))
            .reasoning_content
            .as_deref(),
        Some("A")
    );
}

#[test]
fn unsigned_thinking_normalization_reads_only_truthy_signatures() {
    let signed = r#"[{"type":"thinking","thinking":"A","signature":1}]"#;
    let pieces = split_chunk(&thinking_chunk(signed, "null"));
    assert_eq!(
        delta(pieces.first().expect("reasoning piece")).thinking_blocks,
        Some(blocks(signed))
    );
    let blank = r#"[{"type":"thinking","thinking":"A","signature":""}]"#;
    let blank_pieces = split_chunk(&thinking_chunk(blank, "null"));
    assert_eq!(
        delta(blank_pieces.first().expect("reasoning piece"))
            .reasoning_content
            .as_deref(),
        Some("A")
    );
}

#[test]
fn unsigned_thinking_normalization_leaves_a_single_group_chunk() {
    let thinking = r#"[{"type":"thinking","thinking":"A"}]"#;
    let single_group = chunk(&format!(
        r#"{{"id":"chunk-1","created":7,"model":"m","choices":[{{"index":0,"delta":{{"thinking_blocks":{thinking}}}}}]}}"#
    ));
    let pieces = split_chunk(&single_group);
    assert_eq!(pieces.len(), 1);
    assert_eq!(
        delta(single(&pieces)).thinking_blocks,
        Some(blocks(thinking))
    );
}

// splitter_pull_and_attributes

#[test]
fn splitter_pull_and_attributes_drains_the_buffer_before_a_pull() {
    let combined = chunk(
        r#"{"id":"chunk-1","created":7,"model":"m","choices":[{"index":0,"delta":{"content":"T","reasoning_content":"R","tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}}]}"#,
    );
    let mut split = splitter(TestSource::new(vec![combined]));
    let first = split.next().expect("a piece").expect("a chunk");
    assert_eq!(delta(&first).reasoning_content.as_deref(), Some("R"));
    assert_eq!(split.source.pulls, 1);
    let second = split.next().expect("a piece").expect("a chunk");
    assert_eq!(delta(&second).content.as_deref(), Some("T"));
    assert_eq!(split.source.pulls, 1, "a buffered piece needs no pull");
    let third = split.next().expect("a piece").expect("a chunk");
    assert!(delta(&third).tool_calls.is_some());
    assert_eq!(split.source.pulls, 1);
    assert!(split.next().is_none());
    assert_eq!(split.source.pulls, 2, "exhaustion takes one pull");
}

#[test]
fn splitter_pull_and_attributes_keeps_instances_separate() {
    let combined = text_chunk("Hello", "\"stop\"");
    let mut first = splitter(TestSource::new(vec![combined.clone()]));
    let mut second = splitter(TestSource::new(vec![combined]));
    let first_piece = first.next().expect("a piece").expect("a chunk");
    let second_piece = second.next().expect("a piece").expect("a chunk");
    assert_eq!(delta(&first_piece).content.as_deref(), Some("Hello"));
    assert_eq!(delta(&second_piece).content.as_deref(), Some("Hello"));
    assert_eq!(first.source.pulls, 1);
    assert_eq!(second.source.pulls, 1);
    assert_eq!(first.buffer.len(), 1);
    assert_eq!(second.buffer.len(), 1);
}

#[test]
fn splitter_pull_and_attributes_forwards_stream_attributes() {
    let mut source = TestSource::new(Vec::new());
    source.chunks_attr = Some(vec![text_chunk("Hello", "null")]);
    source.messages_attr = Some(vec![Json::Null]);
    let split = splitter(source);
    assert_eq!(
        ChunkSource::chunks(&split.source).map(<[StreamChunk]>::len),
        Some(1)
    );
    assert_eq!(
        ChunkSource::messages(&split.source).map(<[Json]>::len),
        Some(1)
    );
    let bare = splitter(TestSource::new(Vec::new()));
    assert!(ChunkSource::chunks(&bare.source).is_none());
    assert!(ChunkSource::messages(&bare.source).is_none());
}

#[test]
fn splitter_pull_and_attributes_default_stream_attributes_are_absent() {
    assert!(ChunkSource::chunks(&BareSource).is_none());
    assert!(ChunkSource::messages(&BareSource).is_none());
    assert!(AsyncChunkSource::chunks(&BareSource).is_none());
    assert!(AsyncChunkSource::messages(&BareSource).is_none());
}

#[test]
fn splitter_pull_and_attributes_leaves_source_chunks_unchanged() {
    let combined = text_chunk("Hello", "\"stop\"");
    let before = combined.clone();
    let mut split = splitter(TestSource::new(vec![combined]));
    let _first = split.next();
    let _second = split.next();
    let source_chunk = split
        .source
        .chunks
        .front()
        .expect("the source still holds its chunk");
    assert_eq!(*source_chunk, before);
}

#[test]
fn splitter_pull_and_attributes_propagates_a_source_failure() {
    let failure = StreamFailure::Reported {
        status_code: 429,
        message: "slow down".to_owned(),
    };
    let mut split = splitter(TestSource::failing(failure));
    let error = split
        .next()
        .expect("a failed pull is an item")
        .expect_err("the failure propagates");
    assert_eq!(
        error_status_and_message(&error),
        (429, "slow down".to_owned())
    );
}

#[test]
fn splitter_pull_and_attributes_drains_the_buffer_in_async_mode() {
    let combined = text_chunk("Hello", "\"stop\"");
    let mut split = async_splitter(TestSource::new(vec![combined]));
    current_thread().block_on(async {
        let content = next_split_chunk(&mut split)
            .await
            .expect("a piece")
            .expect("a chunk");
        assert_eq!(delta(&content).content.as_deref(), Some("Hello"));
        assert_eq!(split.source.pulls, 1);
        let finish = next_split_chunk(&mut split)
            .await
            .expect("a piece")
            .expect("a chunk");
        assert_eq!(only_choice(&finish).finish_reason.as_deref(), Some("stop"));
        assert_eq!(split.source.pulls, 1);
        assert!(next_split_chunk(&mut split).await.expect("end").is_none());
        assert_eq!(split.source.pulls, 2);
    });
}

#[test]
fn splitter_pull_and_attributes_propagates_an_async_failure() {
    let failure = StreamFailure::Unclassified {
        text: "stream broke".to_owned(),
    };
    let mut split = async_splitter(TestSource::failing(failure));
    current_thread().block_on(async {
        let error = next_split_chunk(&mut split)
            .await
            .expect_err("the failure propagates");
        assert_eq!(
            error_status_and_message(&error),
            (500, "stream broke".to_owned())
        );
    });
}

#[test]
fn splitter_pull_and_attributes_async_attributes_match_the_source() {
    let mut source = TestSource::new(Vec::new());
    source.chunks_attr = Some(Vec::new());
    source.messages_attr = Some(Vec::new());
    let split = async_splitter(source);
    assert_eq!(
        AsyncChunkSource::chunks(&split.source).map(<[StreamChunk]>::len),
        Some(0)
    );
    assert_eq!(
        AsyncChunkSource::messages(&split.source).map(<[Json]>::len),
        Some(0)
    );
}

// delta_and_error_literals

#[rstest::rstest]
#[case::text(StreamingDeltaType::TextDelta, "text")]
#[case::input_json(StreamingDeltaType::InputJsonDelta, "partial_json")]
#[case::thinking(StreamingDeltaType::ThinkingDelta, "thinking")]
#[case::signature(StreamingDeltaType::SignatureDelta, "signature")]
fn delta_and_error_literals_maps_each_delta_type(
    #[case] delta_type: StreamingDeltaType,
    #[case] field: &str,
) {
    assert_eq!(delta_type.payload_field(), field);
}

#[test]
fn delta_and_error_literals_reads_each_delta_type_from_its_wire_name() {
    let parsed: StreamingDeltaType =
        serde_json::from_str("\"input_json_delta\"").expect("the wire name parses");
    assert_eq!(parsed, StreamingDeltaType::InputJsonDelta);
    assert_eq!(
        serde_json::to_string(&StreamingDeltaType::SignatureDelta)
            .expect("the wire name serializes"),
        "\"signature_delta\""
    );
}

#[test]
fn delta_and_error_literals_keeps_the_reported_status_and_message() {
    let reported = StreamFailure::Reported {
        status_code: 429,
        message: "slow down".to_owned(),
    };
    assert_eq!(
        error_status_and_message(&reported),
        (429, "slow down".to_owned())
    );
    let empty = StreamFailure::Reported {
        status_code: 500,
        message: String::new(),
    };
    assert_eq!(error_status_and_message(&empty), (500, String::new()));
}

#[rstest::rstest]
#[case::text("boom", (500, "boom"))]
#[case::empty_text("", (500, INCOMPLETE_STREAM_MESSAGE))]
fn delta_and_error_literals_falls_back_for_unrelated_failures(
    #[case] text: &str,
    #[case] expected: (u16, &str),
) {
    let failure = StreamFailure::Unclassified {
        text: text.to_owned(),
    };
    assert_eq!(
        error_status_and_message(&failure),
        (expected.0, expected.1.to_owned())
    );
}

#[rstest::rstest]
#[case::internal_error(
    StreamFailure::Reported { status_code: 500, message: "boom".to_owned() },
    r#"{"type": "error", "error": {"type": "api_error", "message": "boom"}}"#
)]
#[case::rate_limit(
    StreamFailure::Reported { status_code: 429, message: "boom".to_owned() },
    r#"{"type": "error", "error": {"type": "rate_limit_error", "message": "boom"}}"#
)]
#[case::unrelated(
    StreamFailure::Unclassified { text: "boom".to_owned() },
    r#"{"type": "error", "error": {"type": "api_error", "message": "boom"}}"#
)]
fn delta_and_error_literals_frames_the_mapped_error(
    #[case] failure: StreamFailure,
    #[case] body: &str,
) {
    let frame = mid_stream_error_sse_event(&failure, LIMITS).expect("the frame is encodable");
    assert_eq!(frame, format!("event: error\ndata: {body}\n\n"));
}
