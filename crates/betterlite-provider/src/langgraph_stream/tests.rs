#![cfg(test)]

use std::collections::VecDeque;
use std::env;
use std::fs;
use std::hint::black_box;
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;

use betterlite_http::error::HttpError;
use betterlite_openai::aleph_alpha::lines::{LineDecoder, decode_text};
use betterlite_openai::stream::chunk::Chunk;
use futures_util::StreamExt as _;
use futures_util::stream;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};
use tracing::Level;
use tracing::subscriber::with_default;
use uuid::Uuid;

use super::error::LangGraphStreamError;
use super::iterator::{LineOutcome, parse_sse_line};
use super::stream::{LangGraphSseStream, LineReader};
use super::{HAS_CUSTOM_STREAM_WRAPPER, IdSource, MAX_LINE_BYTES, MAX_LOGGED_LINE};

const MODEL: &str = "langgraph/agent";

struct FixedIds {
    next: Mutex<u32>,
}

impl FixedIds {
    fn new() -> Self {
        Self {
            next: Mutex::new(0),
        }
    }
}

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        let mut next = self.next.lock().unwrap();
        let id = Uuid::from_u128(u128::from(*next));
        *next = next.checked_add(1).unwrap();
        id
    }
}

fn parse(line: &str) -> Result<LineOutcome, LangGraphStreamError> {
    parse_sse_line(line, MODEL, &FixedIds::new())
}

fn content_line(text: &str) -> String {
    format!("data: {{\"content\": \"{text}\"}}")
}

fn content_of(outcome: LineOutcome) -> Chunk {
    let LineOutcome::Chunk(chunk) = outcome else {
        panic!("expected a content chunk");
    };
    chunk
}

fn final_of(outcome: LineOutcome) -> Chunk {
    let LineOutcome::Final(chunk) = outcome else {
        panic!("expected a final chunk");
    };
    chunk
}

fn assert_field_type(result: &Result<LineOutcome, LangGraphStreamError>, field: &str) {
    let Err(LangGraphStreamError::FieldType { field: found }) = result else {
        panic!("expected a field type error for {field}");
    };
    assert_eq!(*found, field);
}

fn delta_text(chunk: &Chunk) -> Option<&str> {
    chunk.choices[0].delta.content.as_deref()
}

fn finish_reason(chunk: &Chunk) -> Option<&str> {
    chunk.choices[0].finish_reason.as_deref()
}

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

fn capture<T, F>(level: Level, run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let path = env::temp_dir().join(format!(
        "betterlite-langgraph-stream-{}-{}.log",
        process::id(),
        thread::current().name().unwrap_or("test")
    ));
    let log = fs::File::create(&path).unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(level)
        .with_writer(Arc::new(log))
        .finish();
    let result = with_default(subscriber, run);
    let text = fs::read_to_string(&path).unwrap();
    fs::remove_file(&path).unwrap();
    (result, text)
}

fn stream_from(frames: Vec<Result<Vec<u8>, HttpError>>) -> LangGraphSseStream {
    LangGraphSseStream {
        model: String::from(MODEL),
        ids: Arc::new(FixedIds::new()),
        lines: Some(LineReader {
            chunks: stream::iter(frames).boxed(),
            decoder: LineDecoder::new(MAX_LINE_BYTES),
            lines: VecDeque::new(),
        }),
        finished: false,
    }
}

fn stream_of(lines: &[&str]) -> LangGraphSseStream {
    stream_from(vec![Ok(lines.join("\n").into_bytes())])
}

async fn collect(mut stream: LangGraphSseStream) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next_chunk().await {
        chunks.push(chunk);
    }
    chunks
}

#[rstest]
#[case::empty("")]
#[case::spaces("   ")]
#[case::event("event: metadata")]
#[case::comment(": keep-alive")]
#[case::no_colon("data")]
#[case::capital("Data: {\"content\": \"x\"}")]
#[case::space_before_colon("data : {\"content\": \"x\"}")]
fn a_line_that_does_not_start_with_data_gives_nothing(#[case] line: &str) {
    assert!(matches!(parse(line).unwrap(), LineOutcome::Skipped));
}

#[rstest]
#[case::nothing("data:")]
#[case::spaces("data:   ")]
fn an_empty_data_remainder_gives_nothing(#[case] line: &str) {
    assert!(matches!(parse(line).unwrap(), LineOutcome::Skipped));
}

#[rstest]
#[case::object(r#"data: {"content": }"#)]
#[case::list("data: [\"messages\", ]")]
#[case::text("data: not json")]
fn a_line_that_is_not_json_is_skipped(#[case] line: &str) {
    assert!(matches!(parse(line).unwrap(), LineOutcome::Skipped));
}

#[test]
fn a_bad_json_line_logs_its_first_hundred_characters() {
    let line = format!("data: {}", "x".repeat(MAX_LOGGED_LINE + 20));
    let (outcome, log) = capture(Level::DEBUG, || parse(&line));
    assert!(matches!(outcome.unwrap(), LineOutcome::Skipped));
    let logged = format!(
        "Skipping non-JSON SSE line: data: {}",
        "x".repeat(MAX_LOGGED_LINE - 6)
    );
    assert!(log.contains(&logged), "{log}");
    assert!(!log.contains(&"x".repeat(MAX_LOGGED_LINE - 5)), "{log}");
}

#[rstest]
#[case::number("3")]
#[case::text("\"text\"")]
#[case::null("null")]
#[case::truth("true")]
#[case::falsity("false")]
fn a_json_value_that_is_not_a_list_or_object_gives_nothing(#[case] value: &str) {
    assert!(matches!(
        parse(&format!("data: {value}")).unwrap(),
        LineOutcome::Skipped
    ));
}

#[rstest]
#[case::empty("[]")]
#[case::one_event("[\"messages\"]")]
#[case::one_metadata("[\"metadata\"]")]
fn a_list_shorter_than_two_items_gives_nothing(#[case] value: &str) {
    assert!(matches!(
        parse(&format!("data: {value}")).unwrap(),
        LineOutcome::Skipped
    ));
}

#[rstest]
#[case::unknown_type(r#"data: ["values", {"content": "ignored"}]"#)]
#[case::numeric_type(r#"data: [3, {"content": "ignored"}]"#)]
fn an_unknown_event_type_does_not_fall_into_the_dict_branch(#[case] line: &str) {
    assert!(matches!(parse(line).unwrap(), LineOutcome::Skipped));
}

#[rstest]
#[case::ai_tuple(
    r#"data: ["messages", [[{"type": "ai", "content": "hello"}, {}]]]"#,
    "hello"
)]
#[case::chunk_tuple(
    r#"data: ["messages", [[{"type": "AIMessageChunk", "content": "chunk"}, {}]]]"#,
    "chunk"
)]
#[case::ai_dict(r#"data: ["messages", [{"type": "ai", "content": "hello"}]]"#, "hello")]
#[case::chunk_dict(
    r#"data: ["messages", [{"type": "AIMessageChunk", "content": "chunk"}]]"#,
    "chunk"
)]
fn a_messages_event_yields_the_first_ai_content(#[case] line: &str, #[case] text: &str) {
    let chunk = content_of(parse(line).unwrap());
    assert_eq!(delta_text(&chunk), Some(text));
}

#[rstest]
#[case::empty_content_then_ai(
    r#"data: ["messages", [[{"type": "ai", "content": ""}], [{"type": "ai", "content": "later"}]]]"#,
    "later"
)]
#[case::empty_list_item(r#"data: ["messages", [[], [{"type": "ai", "content": "x"}]]]"#, "x")]
#[case::scalar_item(r#"data: ["messages", [3, [{"type": "ai", "content": "x"}]]]"#, "x")]
#[case::null_content_then_ai(
    r#"data: ["messages", [[{"type": "ai", "content": null}], [{"type": "ai", "content": "x"}]]]"#,
    "x"
)]
fn a_messages_event_skips_items_without_ai_content(#[case] line: &str, #[case] expected: &str) {
    let chunk = content_of(parse(line).unwrap());
    assert_eq!(delta_text(&chunk), Some(expected));
}

#[rstest]
#[case::non_dict_message(r#"data: ["messages", [["text"]]]"#)]
#[case::non_ai_type(r#"data: ["messages", [[{"type": "human", "content": "x"}]]]"#)]
#[case::missing_type(r#"data: ["messages", [[{"content": "x"}]]]"#)]
#[case::dict_payload(r#"data: ["messages", {"type": "ai", "content": "x"}]"#)]
#[case::scalar_payload(r#"data: ["messages", "x"]"#)]
fn a_messages_event_without_an_ai_message_gives_nothing(#[case] line: &str) {
    assert!(matches!(parse(line).unwrap(), LineOutcome::Skipped));
}

#[rstest]
#[case::ai_tuple(r#"data: ["messages", [[{"type": "ai", "content": 5}, {}]]]"#)]
#[case::ai_dict(r#"data: ["messages", [{"type": "ai", "content": 5}]]"#)]
#[case::chunk_dict(r#"data: ["messages", [{"type": "AIMessageChunk", "content": 5}]]"#)]
fn a_non_text_message_content_names_the_content_field(#[case] line: &str) {
    assert_field_type(&parse(line), "content");
}

#[rstest]
#[case::run_id(r#"data: ["metadata", {"run_id": "r-1"}]"#)]
#[case::null_run_id(r#"data: ["metadata", {"run_id": null}]"#)]
fn a_metadata_event_with_a_run_id_returns_the_final_chunk(#[case] line: &str) {
    let chunk = final_of(parse(line).unwrap());
    assert_eq!(finish_reason(&chunk), Some("stop"));
    assert_eq!(delta_text(&chunk), None);
}

#[rstest]
#[case::other_keys(r#"data: ["metadata", {"status": "done"}]"#)]
#[case::non_dict(r#"data: ["metadata", "done"]"#)]
fn a_metadata_event_without_a_run_id_gives_nothing(#[case] line: &str) {
    assert!(matches!(parse(line).unwrap(), LineOutcome::Skipped));
}

#[rstest]
#[case::text(r#"data: {"content": "direct"}"#, Some("direct"))]
#[case::null(r#"data: {"content": null}"#, None)]
fn a_dict_event_with_content_yields_that_content(#[case] line: &str, #[case] text: Option<&str>) {
    let chunk = content_of(parse(line).unwrap());
    assert_eq!(delta_text(&chunk), text);
    assert_eq!(chunk.choices[0].delta.role.as_deref(), Some("assistant"));
}

#[rstest]
#[case::number(r#"data: {"content": 5}"#)]
#[case::list(r#"data: {"content": [1]}"#)]
#[case::object(r#"data: {"content": {"a": 1}}"#)]
fn a_non_text_dict_content_names_the_content_field(#[case] line: &str) {
    assert_field_type(&parse(line), "content");
}

#[test]
fn the_content_key_wins_over_the_messages_key() {
    let line = r#"data: {"content": "direct", "messages": [{"type": "ai", "content": "later"}]}"#;
    let chunk = content_of(parse(line).unwrap());
    assert_eq!(delta_text(&chunk), Some("direct"));
}

#[rstest]
#[case::last_ai(
    r#"data: {"messages": [{"type": "human", "content": "q"}, {"type": "ai", "content": "a"}]}"#,
    Some("a")
)]
#[case::empty_content(r#"data: {"messages": [{"type": "ai", "content": ""}]}"#, Some(""))]
#[case::missing_content(r#"data: {"messages": [{"type": "ai"}]}"#, Some(""))]
#[case::null_content(r#"data: {"messages": [{"type": "ai", "content": null}]}"#, None)]
fn a_dict_event_reads_the_last_ai_message(#[case] line: &str, #[case] text: Option<&str>) {
    let chunk = content_of(parse(line).unwrap());
    assert_eq!(delta_text(&chunk), text);
}

#[rstest]
#[case::last_human(
    r#"data: {"messages": [{"type": "ai", "content": "a"}, {"type": "human", "content": "q"}]}"#
)]
#[case::empty_list(r#"data: {"messages": []}"#)]
#[case::null_messages(r#"data: {"messages": null}"#)]
#[case::null_last(r#"data: {"messages": [{"type": "ai", "content": "a"}, null]}"#)]
#[case::no_keys("data: {}")]
fn a_dict_event_without_a_last_ai_message_gives_nothing(#[case] line: &str) {
    assert!(matches!(parse(line).unwrap(), LineOutcome::Skipped));
}

#[test]
fn a_non_list_messages_value_names_the_messages_field() {
    assert_field_type(&parse(r#"data: {"messages": "x"}"#), "messages");
}

#[test]
fn a_non_text_last_message_content_names_the_content_field() {
    assert_field_type(
        &parse(r#"data: {"messages": [{"type": "ai", "content": 5}]}"#),
        "content",
    );
}

#[test]
fn a_content_chunk_carries_the_litellm_envelope() {
    let chunk = content_of(parse(&content_line("hello")).unwrap());
    assert_eq!(chunk.id, "chatcmpl-00000000-0000-0000-0000-000000000000");
    assert_eq!(chunk.object, "chat.completion.chunk");
    assert_eq!(chunk.created, 0);
    assert_eq!(chunk.model, MODEL);
    assert_eq!(chunk.choices.len(), 1);
    assert_eq!(chunk.choices[0].index, 0);
    assert_eq!(chunk.choices[0].delta.role.as_deref(), Some("assistant"));
    assert_eq!(chunk.choices[0].finish_reason, None);
    assert_eq!(chunk.usage, None);
}

#[test]
fn a_final_chunk_carries_an_empty_delta_and_stop() {
    let chunk = final_of(parse(r#"data: ["metadata", {"run_id": "r"}]"#).unwrap());
    assert_eq!(chunk.object, "chat.completion.chunk");
    assert_eq!(chunk.created, 0);
    assert_eq!(chunk.model, MODEL);
    assert_eq!(chunk.choices[0].index, 0);
    assert_eq!(chunk.choices[0].delta.role, None);
    assert_eq!(chunk.choices[0].delta.content, None);
    assert_eq!(chunk.choices[0].finish_reason.as_deref(), Some("stop"));
}

#[test]
fn every_chunk_gets_a_fresh_chatcmpl_id() {
    let ids = FixedIds::new();
    let first = content_of(parse_sse_line(&content_line("a"), MODEL, &ids).unwrap());
    let second = content_of(parse_sse_line(&content_line("b"), MODEL, &ids).unwrap());
    assert_eq!(first.id, "chatcmpl-00000000-0000-0000-0000-000000000000");
    assert_eq!(second.id, "chatcmpl-00000000-0000-0000-0000-000000000001");
}

#[test]
fn the_custom_stream_wrapper_flag_is_set() {
    assert!(black_box(HAS_CUSTOM_STREAM_WRAPPER));
}

#[tokio::test]
async fn before_iteration_starts_next_ends_at_once() {
    let mut stream = LangGraphSseStream {
        model: String::from(MODEL),
        ids: Arc::new(FixedIds::new()),
        lines: None,
        finished: false,
    };
    assert!(stream.next_chunk().await.is_none());
    assert!(stream.next_chunk().await.is_none());
}

#[tokio::test]
async fn a_stream_that_ends_without_metadata_sends_one_final_chunk() {
    let chunks = collect(stream_of(&[&content_line("only")])).await;
    assert_eq!(chunks.len(), 2);
    assert_eq!(delta_text(&chunks[0]), Some("only"));
    assert_eq!(finish_reason(&chunks[0]), None);
    assert_eq!(delta_text(&chunks[1]), None);
    assert_eq!(finish_reason(&chunks[1]), Some("stop"));
}

#[tokio::test]
async fn an_empty_stream_still_sends_one_final_chunk() {
    let chunks = collect(stream_of(&[])).await;
    assert_eq!(chunks.len(), 1);
    assert_eq!(finish_reason(&chunks[0]), Some("stop"));
}

#[tokio::test]
async fn metadata_with_a_run_id_ends_the_stream_with_one_stop_chunk() {
    let chunks = collect(stream_of(&[r#"data: ["metadata", {"run_id": "r-1"}]"#])).await;
    assert_eq!(chunks.len(), 1);
    assert_eq!(finish_reason(&chunks[0]), Some("stop"));
}

#[tokio::test]
async fn content_after_metadata_is_returned_and_no_second_final_chunk_follows() {
    let chunks = collect(stream_of(&[
        r#"data: ["metadata", {"run_id": "r-1"}]"#,
        &content_line("after"),
    ]))
    .await;
    assert_eq!(chunks.len(), 2);
    assert_eq!(finish_reason(&chunks[0]), Some("stop"));
    assert_eq!(delta_text(&chunks[1]), Some("after"));
}

#[tokio::test]
async fn a_line_split_across_two_reads_is_parsed_as_one() {
    let chunks = collect(stream_from(vec![
        Ok(b"data: {\"cont".to_vec()),
        Ok(b"ent\": \"split\"}\n".to_vec()),
    ]))
    .await;
    assert_eq!(chunks.len(), 2);
    assert_eq!(delta_text(&chunks[0]), Some("split"));
}

#[tokio::test]
async fn a_character_split_across_two_reads_is_decoded() {
    let chunks = collect(stream_from(vec![
        Ok(b"data: {\"content\": \"h\xc3".to_vec()),
        Ok(b"\xa9llo\"}\n".to_vec()),
    ]))
    .await;
    assert_eq!(delta_text(&chunks[0]), Some("h\u{e9}llo"));
}

#[test]
fn a_transport_error_mid_stream_is_logged_and_ends_the_stream() {
    let (chunks, log) = capture(Level::ERROR, || {
        runtime().block_on(collect(stream_from(vec![
            Ok(format!("{}\n", content_line("one")).into_bytes()),
            Err(HttpError::Timeout),
        ])))
    });
    assert_eq!(chunks.len(), 1);
    assert_eq!(delta_text(&chunks[0]), Some("one"));
    assert!(
        log.contains("Error in LangGraph SSE stream: the LangGraph stream body could not be read"),
        "{log}"
    );
}

#[test]
fn a_field_type_error_mid_stream_is_logged_and_ends_the_stream() {
    let (chunks, log) = capture(Level::ERROR, || {
        runtime().block_on(collect(stream_of(&[r#"data: {"content": 5}"#])))
    });
    assert!(chunks.is_empty());
    assert!(
        log.contains(
            "Error in LangGraph SSE stream: the LangGraph stream content has the wrong type"
        ),
        "{log}"
    );
}

#[test]
fn replays_the_langgraph_stream_corpus() {
    let seen = fs::read_dir("fuzz/corpus/langgraph_stream")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .map(|path| {
            let line = decode_text(&fs::read(&path).unwrap());
            parse_sse_line(&line, MODEL, &FixedIds::new()).unwrap();
        })
        .count();
    assert!(seen >= 8, "{seen} corpus files");
}
