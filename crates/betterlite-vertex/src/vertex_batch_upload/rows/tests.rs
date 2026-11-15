#![cfg(test)]

use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;

use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use serde_json::Number;

use crate::vertex_batch_upload::line::decode_openai_batch_line;
use crate::vertex_batch_upload::rows::embeddings;
use crate::vertex_batch_upload::sources::{FileContent, FileTypes};
use crate::vertex_batch_upload::tests::upload;

async fn rows_of(line: &str) -> Vec<String> {
    let file = FileTypes::Content(FileContent::Text(line.to_owned()));
    let chunks = upload(file, "batch", &Json::Bool(false), usize::MAX).await;
    let bytes: Vec<u8> = chunks.into_iter().flat_map(Result::unwrap).collect();
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

async fn first_failure(line: &str) -> String {
    let file = FileTypes::Content(FileContent::Text(line.to_owned()));
    let chunks = upload(file, "batch", &Json::Bool(false), usize::MAX).await;
    chunks
        .into_iter()
        .find_map(Result::err)
        .unwrap()
        .to_string()
}

#[rstest]
#[case::embeddings_fanout(
    r#"{"custom_id":"a#0/2","url":"/v1/embeddings","body":{"model":"m","input":["x","y"]}}"#,
    &[
        r#"{"key": "a%230%2F2#0/2", "request": {"content": {"parts": [{"text": "x"}]}}}"#,
        r#"{"key": "a%230%2F2#1/2", "request": {"content": {"parts": [{"text": "y"}]}}}"#,
    ]
)]
#[case::combined_inner_input(
    r#"{"custom_id":"c","url":"/v1/embeddings","body":{"input":[["x","y"],"z"]}}"#,
    &[
        r#"{"key": "c#0/2", "request": {"content": {"parts": [{"text": "x"}, {"text": "y"}]}}}"#,
        r#"{"key": "c#1/2", "request": {"content": {"parts": [{"text": "z"}]}}}"#,
    ]
)]
#[case::single_input_has_no_position(
    r#"{"custom_id":"s/ 1","url":"embeddings","body":{"input":["x"]}}"#,
    &[r#"{"key": "s%2F%201", "request": {"content": {"parts": [{"text": "x"}]}}}"#]
)]
#[case::ascii_json_bytes(
    r#"{"custom_id":"só","url":"/v1/embeddings","body":{"input":"é😀"}}"#,
    &[r#"{"key": "s%C3%B3", "request": {"content": {"parts": [{"text": "\u00e9\ud83d\ude00"}]}}}"#]
)]
#[case::no_custom_id_no_key(
    r#"{"url":"/v1/embeddings","body":{"input":""}}"#,
    &[r#"{"request": {"content": {"parts": [{"text": ""}]}}}"#]
)]
#[case::config_fields_in_order(
    r#"{"url":"/v1/embeddings","body":{"input":"x","title":"T","task_type":"RETRIEVAL_QUERY","dimensions":8,"model":"m"}}"#,
    &[r#"{"request": {"content": {"parts": [{"text": "x"}]}, "output_dimensionality": 8, "task_type": "RETRIEVAL_QUERY", "title": "T"}}"#]
)]
#[tokio::test]
async fn embedding_rows(#[case] line: &str, #[case] expected: &[&str]) {
    assert_eq!(rows_of(line).await, expected);
}

#[rstest]
#[case::empty_embeddings(
    r#"{"url":"/v1/embeddings","body":{"input":[]}}"#,
    "`input` on /v1/embeddings batch requests must not be empty"
)]
#[case::missing_input(
    r#"{"url":"/v1/embeddings","body":{"model":"m"}}"#,
    "`input` is required on /v1/embeddings batch requests, but was not provided"
)]
#[case::null_input(
    r#"{"url":"/v1/embeddings","body":{"input":null}}"#,
    "`input` is required on /v1/embeddings batch requests, but was not provided"
)]
#[case::missing_body(r#"{"url":"/v1/embeddings"}"#, "`body` must be a JSON object")]
#[case::body_not_an_object(
    r#"{"url":"/v1/embeddings","body":[1]}"#,
    "`body` must be a JSON object"
)]
#[case::chat_body_not_an_object(
    r#"{"url":"/v1/chat/completions","body":5}"#,
    "`body` must be a JSON object"
)]
#[tokio::test]
async fn rejected_rows(#[case] line: &str, #[case] message: &str) {
    assert_eq!(first_failure(line).await, message);
}

#[rstest]
#[case::plain(r#""/v1/embeddings""#, true)]
#[case::bare(r#""embeddings""#, true)]
#[case::trailing_slash(r#""/v1/embeddings/""#, true)]
#[case::query_with_slash(r#""/v1/embeddings?x=1/""#, true)]
#[case::slash_before_query(r#""/v1/embeddings/?x=1""#, true)]
#[case::relative_suffix(r#""v1/embeddings""#, true)]
#[case::longer_name(r#""/v1/myembeddings""#, false)]
#[case::chat(r#""/v1/chat/completions""#, false)]
#[case::number_url("5", false)]
#[case::null_url("null", false)]
#[tokio::test]
async fn query_route(#[case] url: &str, #[case] embeddings: bool) {
    let body =
        r#"{"model":"gemini-2.0-flash","input":["x"],"messages":[{"role":"user","content":"hi"}]}"#;
    let rows = rows_of(&format!(r#"{{"url":{url},"body":{body}}}"#)).await;
    assert_eq!(rows.len(), 1);
    let text = rows.first().unwrap();
    assert_eq!(text.contains(r#""content": {"parts""#), embeddings);
    assert_eq!(text.contains(r#""contents""#), !embeddings);
}

#[rstest]
#[case::slash_url("/v1/responses")]
#[case::query_url("/v1/responses?x=1/")]
#[case::bare_url("responses")]
#[tokio::test]
async fn responses_route(#[case] url: &str) {
    let line =
        format!(r#"{{"url":"{url}","body":{{"model":"gemini-2.0-flash","input":"hello"}}}}"#);
    let rows = rows_of(&line).await;
    assert!(rows.first().unwrap().contains(r#""text": "hello""#));
}

#[tokio::test]
async fn developer_role() {
    let line = r#"{"url":"/v1/chat/completions","body":{"model":"gemini-2.0-flash","messages":[{"role":"developer","content":"be brief","name":"ops"},{"role":"user","content":"hi"}]}}"#;
    let rows = rows_of(line).await;
    let text = rows.first().unwrap();
    assert!(text.contains(r#""system_instruction""#));
    assert!(text.contains("be brief"));
    assert!(!text.contains("ops"));
}

#[rstest]
#[case::attached(
    r#"{"custom_id":"abc","url":"/v1/chat/completions","body":{"model":"gemini-2.0-flash","messages":[{"role":"user","content":"hi"}]}}"#,
    Some(r#""labels": {"litellm_custom_id": "abc", "litellm_custom_id_raw": "b32_mfrgg"}"#)
)]
#[case::absent_without_custom_id(
    r#"{"url":"/v1/chat/completions","body":{"model":"gemini-2.0-flash","messages":[{"role":"user","content":"hi"}]}}"#,
    None
)]
#[tokio::test]
async fn label_attachment(#[case] line: &str, #[case] labels: Option<&str>) {
    let rows = rows_of(line).await;
    let text = rows.first().unwrap();
    assert_eq!(text.contains(r#""labels""#), labels.is_some());
    if let Some(expected) = labels {
        assert!(text.contains(expected));
    }
}

#[rstest]
#[case::temperature(r#""temperature":0.5"#, r#""temperature": 0.5"#)]
#[case::max_tokens(r#""max_tokens":16"#, r#""max_output_tokens": 16"#)]
#[tokio::test]
async fn chat_options(#[case] option: &str, #[case] expected: &str) {
    let body = r#""model":"gemini-2.0-flash","messages":[{"role":"user","content":"hi"}]"#;
    let rows = rows_of(&format!(
        r#"{{"url":"/v1/chat/completions","body":{{{body},{option}}}}}"#
    ))
    .await;
    assert!(rows.first().unwrap().contains(expected));
}

#[rstest]
#[case::literally_true("batch", Json::Bool(true), true)]
#[case::the_number_one("batch", Json::Number(Number::from(1_i32)), false)]
#[case::the_text_true("batch", Json::Text("true".to_owned()), false)]
#[case::false_flag("batch", Json::Bool(false), false)]
#[case::null_flag("batch", Json::Null, false)]
#[case::other_purpose("assistants", Json::Bool(true), false)]
#[tokio::test]
async fn literal_passthrough_flag(
    #[case] purpose: &str,
    #[case] passthrough: Json,
    #[case] raw: bool,
) {
    let line = r#"{"url":"/v1/embeddings","body":{"input":"x"}}"#;
    let file = FileTypes::Content(FileContent::Text(line.to_owned()));
    let chunks = upload(file, purpose, &passthrough, usize::MAX).await;
    let bytes: Vec<u8> = chunks.into_iter().flat_map(Result::unwrap).collect();
    assert_eq!(bytes == line.as_bytes(), raw);
}

#[rstest]
#[case::blank(b"  \r\n".as_slice(), None)]
#[case::object(br#" {"custom_id":"a","url":"u"} "#.as_slice(), Some("a"))]
fn decoded_lines(#[case] line: &[u8], #[case] custom_id: Option<&str>) {
    let decoded = decode_openai_batch_line(line).unwrap();
    assert_eq!(
        decoded.and_then(|entry| entry.custom_id).as_deref(),
        custom_id
    );
}

#[rstest]
#[case::not_utf8(b"\xff\xfe".as_slice(), "a batch line is not UTF-8")]
#[case::wrong_custom_id_type(br#"{"custom_id":5}"#.as_slice(), "the batch line value at `custom_id` is not valid")]
#[case::trailing_text(br#"{"custom_id":"a"} x"#.as_slice(), "the batch line value at `.` is not valid")]
#[case::array(br#"["a"]"#.as_slice(), "a batch line must be a JSON object")]
#[case::text(br#""a""#.as_slice(), "the batch line value at `.` is not valid")]
fn undecodable_lines(#[case] line: &[u8], #[case] message: &str) {
    assert_eq!(
        decode_openai_batch_line(line).unwrap_err().to_string(),
        message
    );
}

#[rstest]
fn replays_the_vertex_batch_upload_corpus(
    #[files("fuzz/corpus/vertex_batch_upload/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if let Ok(Some(line)) = decode_openai_batch_line(&bytes) {
        drop(embeddings::rows::<Infallible>(&line));
    }
}
