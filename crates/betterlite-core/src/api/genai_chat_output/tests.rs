#![cfg(test)]

use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::stream::chunk::Chunk;
use futures_util::stream::{self, StreamExt as _};
use rstest::rstest;

use super::{
    GenAiOutputError, GenAiStreamState, translate_chunk, translate_completion, translate_stream,
};

const COMPLETION: &str = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"m","choices":[{"index":0,"message":MESSAGE,"finish_reason":FINISH}],"usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}}"#;
const CHUNK: &str = r#"{"id":"chunk-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[{"index":0,"delta":DELTA,"finish_reason":FINISH}],"usage":USAGE}"#;
const TOOL_CALL_MESSAGE: &str = r#"{"role":"assistant","content":CONTENT,"tool_calls":[{"id":"call_1","type":"function","function":{"name":NAME,"arguments":ARGUMENTS}}]}"#;
const TEXT_MESSAGE: &str = r#"{"role":"assistant","content":"hi"}"#;
const USAGE_1_2_3: &str = r#"{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}"#;

fn chat_response(body: &str) -> ChatResponse {
    serde_json::from_str(body).expect("a chat completion response")
}

fn completion(message: &str, finish_reason: &str) -> ChatResponse {
    chat_response(
        &COMPLETION
            .replace("MESSAGE", message)
            .replace("FINISH", finish_reason),
    )
}

fn tool_call_response(content: &str, name: &str, arguments: &str) -> ChatResponse {
    let message = TOOL_CALL_MESSAGE
        .replace("CONTENT", content)
        .replace("NAME", name)
        .replace("ARGUMENTS", arguments);
    completion(&message, r#""tool_calls""#)
}

fn translated(response: &ChatResponse) -> String {
    serde_json::to_string(&translate_completion(response).expect("the translated response"))
        .expect("the response serializes")
}

fn completion_parts(response: &ChatResponse) -> String {
    let translated = translate_completion(response).expect("the translated response");
    serde_json::to_string(&translated.candidates[0].content.parts).expect("the parts serialize")
}

fn stream_chunk(delta: &str, finish_reason: Option<&str>, usage: Option<&str>) -> Chunk {
    let finish = serde_json::to_string(&finish_reason).expect("a finish reason");
    let body = CHUNK
        .replace("DELTA", delta)
        .replace("FINISH", &finish)
        .replace("USAGE", usage.unwrap_or("null"));
    serde_json::from_str(&body).expect("a stream chunk")
}

fn tool_chunk(index: u32, name: Option<&str>, arguments: &str) -> Chunk {
    let name_json = serde_json::to_string(&name).expect("a tool name");
    let arguments_json = serde_json::to_string(arguments).expect("tool arguments");
    let delta = format!(
        r#"{{"tool_calls":[{{"index":{index},"function":{{"name":{name_json},"arguments":{arguments_json}}}}}]}}"#
    );
    stream_chunk(&delta, None, None)
}

fn translated_chunk(chunk: &Chunk, state: &mut GenAiStreamState) -> String {
    serde_json::to_string(&translate_chunk(chunk, state).expect("a translated chunk"))
        .expect("the chunk serializes")
}

fn chunk_parts(chunk: &Chunk, state: &mut GenAiStreamState) -> String {
    let translated = translate_chunk(chunk, state).expect("a translated chunk");
    serde_json::to_string(&translated.candidates[0].content.parts).expect("the parts serialize")
}

async fn framed(chunks: Vec<Chunk>) -> Vec<String> {
    translate_stream(stream::iter(chunks).boxed())
        .map(|frame| String::from_utf8(frame.expect("a framed chunk")).expect("utf-8"))
        .collect()
        .await
}

#[test]
fn a_completion_maps_the_first_choice_message_finish_reason_and_usage() {
    let response = chat_response(
        r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"m","choices":[
            {"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"},
            {"index":1,"message":{"role":"assistant","content":"second"},"finish_reason":"length"}
        ],"usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}}"#,
    );
    assert_eq!(
        translated(&response),
        r#"{"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":"STOP","index":0,"safetyRatings":[]}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":5,"totalTokenCount":8}}"#
    );
}

#[test]
fn a_completion_without_choices_is_an_error() {
    let response = chat_response(
        r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,"model":"m","choices":[]}"#,
    );
    assert!(matches!(
        translate_completion(&response),
        Err(GenAiOutputError::NoChoices)
    ));
}

#[test]
fn a_message_without_text_or_calls_yields_one_empty_text_part() {
    let response = completion(r#"{"role":"assistant","content":null}"#, r#""stop""#);
    assert_eq!(completion_parts(&response), r#"[{"text":""}]"#);
}

#[test]
fn message_text_and_tool_calls_become_text_and_function_call_parts() {
    let response = tool_call_response(r#""calling""#, r#""lookup""#, r#""{\"city\":\"Paris\"}""#);
    assert_eq!(
        completion_parts(&response),
        r#"[{"text":"calling"},{"functionCall":{"name":"lookup","args":{"city":"Paris"}}}]"#
    );
}

#[test]
fn empty_message_text_is_dropped_before_tool_calls() {
    let response = tool_call_response(r#""""#, r#""lookup""#, r#""{}""#);
    assert_eq!(
        completion_parts(&response),
        r#"[{"functionCall":{"name":"lookup","args":{}}}]"#
    );
}

#[rstest]
#[case::empty(r#""""#, "{}")]
#[case::malformed(r#""{not json""#, "{}")]
#[case::object(r#""{\"city\":\"Paris\"}""#, r#"{"city":"Paris"}"#)]
#[case::scalar(r#""5""#, "5")]
fn a_tool_call_maps_its_arguments(#[case] arguments: &str, #[case] expected: &str) {
    let response = tool_call_response("null", r#""lookup""#, arguments);
    assert_eq!(
        completion_parts(&response),
        format!(r#"[{{"functionCall":{{"name":"lookup","args":{expected}}}}}]"#)
    );
}

#[test]
fn a_tool_call_with_a_missing_name_becomes_undefined_tool_name() {
    let response = tool_call_response("null", r#""""#, r#""{}""#);
    assert_eq!(
        completion_parts(&response),
        r#"[{"functionCall":{"name":"undefined_tool_name","args":{}}}]"#
    );
}

#[rstest]
#[case::stop("stop", "STOP")]
#[case::length("length", "MAX_TOKENS")]
#[case::content_filter("content_filter", "SAFETY")]
#[case::tool_calls("tool_calls", "STOP")]
#[case::function_call("function_call", "STOP")]
#[case::unmapped("weird", "STOP")]
#[case::empty("", "STOP")]
fn finish_reasons_map_to_google_genai_reasons(#[case] reason: &str, #[case] expected: &str) {
    let response = completion(TEXT_MESSAGE, &format!("\"{reason}\""));
    let translated = translate_completion(&response).expect("the translated response");
    assert_eq!(translated.candidates[0].finish_reason, Some(expected));
}

#[test]
fn an_absent_finish_reason_maps_to_stop() {
    let response = completion(TEXT_MESSAGE, "null");
    let translated = translate_completion(&response).expect("the translated response");
    assert_eq!(translated.candidates[0].finish_reason, Some("STOP"));
}

#[test]
fn a_text_delta_becomes_one_text_part() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk(r#"{"content":"Hello"}"#, None, None);
    assert_eq!(
        translated_chunk(&chunk, &mut state),
        r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"},"finishReason":null,"index":0,"safetyRatings":[]}]}"#
    );
}

#[test]
fn an_empty_delta_produces_no_chunk() {
    let mut state = GenAiStreamState::default();
    assert!(translate_chunk(&stream_chunk("{}", None, None), &mut state).is_none());
}

#[test]
fn an_empty_delta_with_a_finish_reason_produces_a_chunk() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk("{}", Some("stop"), None);
    assert_eq!(
        translated_chunk(&chunk, &mut state),
        r#"{"candidates":[{"content":{"parts":[],"role":"model"},"finishReason":"STOP","index":0,"safetyRatings":[]}],"usageMetadata":{"promptTokenCount":0,"candidatesTokenCount":0,"totalTokenCount":0}}"#
    );
}

#[test]
fn an_empty_content_string_produces_no_part() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk(r#"{"content":""}"#, None, None);
    assert!(translate_chunk(&chunk, &mut state).is_none());
}

#[test]
fn a_chunk_without_choices_produces_nothing() {
    let chunk: Chunk = serde_json::from_str(
        r#"{"id":"chunk-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[]}"#,
    )
    .expect("a stream chunk");
    let mut state = GenAiStreamState::default();
    assert!(translate_chunk(&chunk, &mut state).is_none());
}

#[test]
fn streaming_tool_calls_accumulate_by_index() {
    let mut state = GenAiStreamState::default();
    let first = tool_chunk(0, Some("lookup"), r#"{"ci"#);
    let second = tool_chunk(0, None, r#"ty": "Par"#);
    let third = tool_chunk(0, None, r#"is"}"#);
    assert!(translate_chunk(&first, &mut state).is_none());
    assert!(translate_chunk(&second, &mut state).is_none());
    assert_eq!(
        chunk_parts(&third, &mut state),
        r#"[{"functionCall":{"name":"lookup","args":{"city":"Paris"}}}]"#
    );
    assert!(state.accumulated_tool_calls.is_empty());
}

#[test]
fn a_tool_call_waits_for_its_name_before_it_emits() {
    let mut state = GenAiStreamState::default();
    let arguments = tool_chunk(0, None, r#"{"a":1}"#);
    assert!(translate_chunk(&arguments, &mut state).is_none());
    let name = tool_chunk(0, Some("f"), "");
    assert_eq!(
        chunk_parts(&name, &mut state),
        r#"[{"functionCall":{"name":"f","args":{"a":1}}}]"#
    );
}

#[test]
fn a_later_name_replaces_the_accumulated_one_and_an_empty_name_keeps_it() {
    let mut state = GenAiStreamState::default();
    let first_name = tool_chunk(0, Some("a"), r#"{"x":"#);
    assert!(translate_chunk(&first_name, &mut state).is_none());
    let second_name = tool_chunk(0, Some("b"), "");
    assert!(translate_chunk(&second_name, &mut state).is_none());
    let empty_name = tool_chunk(0, Some(""), "1}");
    assert_eq!(
        chunk_parts(&empty_name, &mut state),
        r#"[{"functionCall":{"name":"b","args":{"x":1}}}]"#
    );
}

#[test]
fn two_tool_calls_accumulate_separately_by_index() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk(
        r#"{"tool_calls":[
            {"index":0,"function":{"name":"a","arguments":"{\"x\":1}"}},
            {"index":1,"function":{"name":"b","arguments":"{\"y\":2}"}}
        ]}"#,
        None,
        None,
    );
    assert_eq!(
        chunk_parts(&chunk, &mut state),
        r#"[{"functionCall":{"name":"a","args":{"x":1}}},{"functionCall":{"name":"b","args":{"y":2}}}]"#
    );
}

#[test]
fn malformed_arguments_wait_for_a_later_chunk() {
    let mut state = GenAiStreamState::default();
    let broken = tool_chunk(0, Some("f"), r#"{"a":"#);
    assert!(translate_chunk(&broken, &mut state).is_none());
    let fixed = tool_chunk(0, None, "1}");
    assert_eq!(
        chunk_parts(&fixed, &mut state),
        r#"[{"functionCall":{"name":"f","args":{"a":1}}}]"#
    );
}

#[test]
fn arguments_that_never_parse_are_dropped_at_end_of_stream() {
    let mut state = GenAiStreamState::default();
    let broken = tool_chunk(0, Some("f"), r#"{"a":"#);
    assert!(translate_chunk(&broken, &mut state).is_none());
    let finish = stream_chunk("{}", Some("stop"), None);
    assert_eq!(chunk_parts(&finish, &mut state), "[]");
}

#[test]
fn a_completed_tool_call_starts_fresh_at_the_same_index() {
    let mut state = GenAiStreamState::default();
    let first = tool_chunk(0, Some("a"), r#"{"x":1}"#);
    assert_eq!(
        chunk_parts(&first, &mut state),
        r#"[{"functionCall":{"name":"a","args":{"x":1}}}]"#
    );
    let second = tool_chunk(0, Some("b"), r#"{"y":2}"#);
    assert_eq!(
        chunk_parts(&second, &mut state),
        r#"[{"functionCall":{"name":"b","args":{"y":2}}}]"#
    );
}

#[test]
fn tool_call_deltas_without_a_function_are_ignored() {
    let mut state = GenAiStreamState::default();
    let custom = stream_chunk(
        r#"{"tool_calls":[{"index":0,"type":"custom"}]}"#,
        None,
        None,
    );
    assert!(translate_chunk(&custom, &mut state).is_none());
    assert!(state.accumulated_tool_calls.is_empty());
    let empty = tool_chunk(0, None, "");
    assert!(translate_chunk(&empty, &mut state).is_none());
    assert!(state.accumulated_tool_calls.is_empty());
}

#[test]
fn usage_is_added_only_to_the_final_chunk() {
    let mut state = GenAiStreamState::default();
    let text = stream_chunk(r#"{"content":"hi"}"#, None, Some(USAGE_1_2_3));
    assert_eq!(
        translated_chunk(&text, &mut state),
        r#"{"candidates":[{"content":{"parts":[{"text":"hi"}],"role":"model"},"finishReason":null,"index":0,"safetyRatings":[]}]}"#
    );
    let finish = stream_chunk("{}", Some("length"), Some(USAGE_1_2_3));
    assert_eq!(
        translated_chunk(&finish, &mut state),
        r#"{"candidates":[{"content":{"parts":[],"role":"model"},"finishReason":"MAX_TOKENS","index":0,"safetyRatings":[]}],"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":2,"totalTokenCount":3}}"#
    );
}

#[test]
fn an_empty_finish_reason_is_ignored() {
    let mut state = GenAiStreamState::default();
    let chunk = stream_chunk("{}", Some(""), None);
    assert!(translate_chunk(&chunk, &mut state).is_none());
}

#[test]
fn only_the_first_streaming_choice_is_translated() {
    let chunk: Chunk = serde_json::from_str(
        r#"{"id":"chunk-1","object":"chat.completion.chunk","created":1,"model":"m","choices":[
            {"index":0,"delta":{},"finish_reason":null},
            {"index":1,"delta":{"content":"second"},"finish_reason":null}
        ]}"#,
    )
    .expect("a stream chunk");
    let mut state = GenAiStreamState::default();
    assert!(translate_chunk(&chunk, &mut state).is_none());
}

#[tokio::test]
async fn the_streaming_entry_point_frames_each_chunk_as_sse_data() {
    let frames = framed(vec![
        stream_chunk(r#"{"content":"a"}"#, None, None),
        stream_chunk("{}", Some("stop"), None),
    ])
    .await;
    assert_eq!(
        frames,
        [
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"a\"}],\"role\":\"model\"},\"finishReason\":null,\"index\":0,\"safetyRatings\":[]}]}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[],\"role\":\"model\"},\"finishReason\":\"STOP\",\"index\":0,\"safetyRatings\":[]}],\"usageMetadata\":{\"promptTokenCount\":0,\"candidatesTokenCount\":0,\"totalTokenCount\":0}}\n\n",
        ]
    );
}

#[tokio::test]
async fn empty_chunks_are_skipped_between_frames() {
    let frames = framed(vec![
        stream_chunk(r#"{"content":"a"}"#, None, None),
        stream_chunk("{}", None, None),
        stream_chunk(r#"{"content":"b"}"#, None, None),
    ])
    .await;
    assert_eq!(frames.len(), 2);
    assert!(frames[1].contains(r#""parts":[{"text":"b"}]"#));
}

#[tokio::test]
async fn each_frame_is_yielded_before_the_next_chunk_is_read() {
    let read = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&read);
    let source = stream::iter(vec![
        stream_chunk(r#"{"content":"a"}"#, None, None),
        stream_chunk(r#"{"content":"b"}"#, None, None),
    ])
    .inspect(move |_chunk| {
        counter.fetch_add(1, Ordering::SeqCst);
    })
    .boxed();
    let mut frames = translate_stream(source);
    assert_eq!(read.load(Ordering::SeqCst), 0);
    let first = frames
        .next()
        .await
        .expect("the first frame")
        .expect("a frame");
    assert_eq!(read.load(Ordering::SeqCst), 1);
    let text = String::from_utf8(first).expect("utf-8");
    assert!(text.contains(r#""parts":[{"text":"a"}]"#));
}

#[rstest]
fn replays_the_genai_chat_output_corpus(#[files("fuzz/corpus/genai_chat_output/*")] path: PathBuf) {
    let bytes = fs::read(&path).expect("a corpus file");
    if path.file_stem() == Some(OsStr::new("invalid-json")) {
        drop(
            serde_json::from_slice::<Chunk>(&bytes)
                .expect_err("a corpus file whose JSON does not parse"),
        );
        return;
    }
    let chunk =
        serde_json::from_slice::<Chunk>(&bytes).expect("every other corpus file is a chunk");
    let mut state = GenAiStreamState::default();
    drop(translate_chunk(&chunk, &mut state));
}
