use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::support::{GoneCounter, SequenceIds, WordsCounter, request_data};
use crate::ollama_chat::reply::{AssistantMessage, Function, OllamaChatResponse, ToolCall};
use crate::ollama_chat::usage::{TokenCounter, Usage};
use crate::ollama_generate::error::OllamaGenerateError;
use crate::ollama_generate::reply::{ReplyInput, transform_response};

const CREATED: i64 = 1_700_000_000;

struct Found {
    message: AssistantMessage,
    finish_reason: String,
    usage: Usage,
}

async fn transform(
    body: &str,
    data: &IndexMap<String, Json>,
    counter: &dyn TokenCounter,
) -> Result<OllamaChatResponse, OllamaGenerateError> {
    let input = ReplyInput {
        model: "llama3",
        body: body.as_bytes(),
        request_data: data,
        created: CREATED,
    };
    transform_response(&input, counter, &SequenceIds::new()).await
}

async fn found(body: &str, data: &IndexMap<String, Json>) -> Found {
    let response = transform(body, data, &WordsCounter).await.unwrap();
    let choice = response.choices.into_iter().next().unwrap();
    Found {
        message: choice.message,
        finish_reason: choice.finish_reason,
        usage: response.usage,
    }
}

async fn failed(body: &str, data: &IndexMap<String, Json>) -> OllamaGenerateError {
    transform(body, data, &WordsCounter).await.unwrap_err()
}

fn nothing() -> IndexMap<String, Json> {
    IndexMap::new()
}

fn json_mode() -> IndexMap<String, Json> {
    request_data(r#"{"format":"json","prompt":"a b"}"#)
}

#[tokio::test]
async fn a_plain_reply_carries_the_model_and_created_second() {
    let data = nothing();
    let input = ReplyInput {
        model: "llama3",
        body: br#"{"response":"hello"}"#,
        request_data: &data,
        created: CREATED,
    };
    let response = transform_response(&input, &WordsCounter, &SequenceIds::new())
        .await
        .unwrap();
    assert_eq!(response.model, "ollama/llama3");
    assert_eq!(response.created, CREATED);
    assert_eq!(response.choices.len(), 1_usize);
    assert_eq!(response.choices[0].index, 0_u32);
    assert_eq!(response.choices[0].message.role, "assistant");
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("hello")
    );
    assert_eq!(response.choices[0].message.reasoning_content, None);
    assert_eq!(response.choices[0].message.tool_calls, None);
    assert_eq!(response.choices[0].finish_reason, "stop");
}

#[rstest]
#[case::plain(r#"{"response":"hello"}"#, Some("hello"), None)]
#[case::empty(r#"{"response":""}"#, Some(""), None)]
#[case::absent("{}", Some(""), None)]
#[case::null(r#"{"response":null}"#, None, None)]
#[case::thinking(
    r#"{"response":"answer","thinking":"why"}"#,
    Some("answer"),
    Some("why")
)]
#[case::empty_thinking_falls_back_to_tags(
    r#"{"response":"answer","thinking":""}"#,
    Some("answer"),
    None
)]
#[case::empty_thinking_with_tags(
    r#"{"response":"<think>a</think>b","thinking":""}"#,
    Some("b"),
    Some("a")
)]
#[case::tags(
    r#"{"response":"<think>why</think>answer"}"#,
    Some("answer"),
    Some("why")
)]
#[case::other_tags(
    r#"{"response":"<thinking>why</thinking>answer"}"#,
    Some("answer"),
    Some("why")
)]
#[case::mid_text_tags(
    r#"{"response":"a<think>b</think>c"}"#,
    Some("a<think>b</think>c"),
    None
)]
#[tokio::test]
async fn a_reply_splits_reasoning_like_litellm(
    #[case] body: &str,
    #[case] content: Option<&str>,
    #[case] reasoning: Option<&str>,
) {
    let found = found(body, &nothing()).await;
    assert_eq!(found.message.content.as_deref(), content);
    assert_eq!(found.message.reasoning_content.as_deref(), reasoning);
    assert_eq!(found.finish_reason, "stop");
    assert_eq!(
        found.usage,
        Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        }
    );
}

#[tokio::test]
async fn the_reply_counts_are_used_when_present() {
    let found = found(
        r#"{"response":"hello","prompt_eval_count":3,"eval_count":4}"#,
        &nothing(),
    )
    .await;
    assert_eq!(
        found.usage,
        Usage {
            prompt_tokens: 3,
            completion_tokens: 4,
            total_tokens: 7,
        }
    );
}

#[tokio::test]
async fn a_missing_prompt_count_falls_back_to_the_counter() {
    let found = found(
        r#"{"response":"hello"}"#,
        &request_data(r#"{"prompt":"a b c"}"#),
    )
    .await;
    assert_eq!(
        found.usage,
        Usage {
            prompt_tokens: 3,
            completion_tokens: 0,
            total_tokens: 3,
        }
    );
}

#[tokio::test]
async fn a_missing_completion_count_uses_the_message_character_count() {
    let found = found(
        r#"{"response":"hello","message":{"content":"abcd"}}"#,
        &nothing(),
    )
    .await;
    assert_eq!(
        found.usage,
        Usage {
            prompt_tokens: 0,
            completion_tokens: 4,
            total_tokens: 4,
        }
    );
}

#[rstest]
#[case::null_prompt(r#"{"response":"hello","prompt_eval_count":null}"#)]
#[case::null_eval(r#"{"response":"hello","eval_count":null}"#)]
#[case::null_message_content(r#"{"response":"hello","message":{"content":null}}"#)]
#[case::null_message(r#"{"response":"hello","message":null}"#)]
#[case::null_content_with_eval_count(
    r#"{"response":"hello","eval_count":4,"message":{"content":null}}"#
)]
#[case::null_message_with_counts(
    r#"{"response":"hello","eval_count":4,"prompt_eval_count":1,"message":null}"#
)]
#[tokio::test]
async fn a_null_count_is_a_reply_field_error(#[case] body: &str) {
    let error = failed(body, &request_data(r#"{"prompt":"a b"}"#)).await;
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::empty_list(r#"{"response":"hello","message":[]}"#)]
#[case::list_with_content(r#"{"response":"hello","message":["abcd"]}"#)]
#[case::list_with_eval_count(r#"{"response":"hello","eval_count":4,"message":[]}"#)]
#[tokio::test]
async fn a_non_object_message_is_a_reply_field_error(#[case] body: &str) {
    let error = failed(body, &request_data(r#"{"prompt":"a b"}"#)).await;
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_counter_without_an_answer_counts_zero() {
    let response = transform(
        r#"{"response":"hello"}"#,
        &request_data(r#"{"prompt":"a b"}"#),
        &GoneCounter,
    )
    .await
    .unwrap();
    assert_eq!(response.usage.prompt_tokens, 0_u64);
}

#[rstest]
#[case::object(r#"{"response":"{\"a\":1}"}"#, "{\"a\":1}")]
#[case::list(r#"{"response":"[1, 2]"}"#, "[1,2]")]
#[case::number(r#"{"response":"5"}"#, "5")]
#[case::text(r#"{"response":"\"text\""}"#, "\"text\"")]
#[case::whitespace(r#"{"response":"   "}"#, "")]
#[case::absent("{}", "")]
#[case::null(r#"{"response":null}"#, "")]
#[tokio::test]
async fn json_mode_content_is_the_parsed_response_written_back(
    #[case] body: &str,
    #[case] content: &str,
) {
    let found = found(body, &json_mode()).await;
    assert_eq!(found.message.content.as_deref(), Some(content));
    assert_eq!(found.finish_reason, "stop");
    assert_eq!(found.message.tool_calls, None);
}

#[tokio::test]
async fn json_mode_sends_a_named_call_as_a_tool_call() {
    let found = found(
        r#"{"response":"{\"name\":\"f\",\"arguments\":{\"x\":1}}"}"#,
        &json_mode(),
    )
    .await;
    assert_eq!(found.finish_reason, "tool_calls");
    assert_eq!(found.message.content, None);
    assert_eq!(
        found.message.tool_calls,
        Some(vec![ToolCall {
            id: String::from("call_id-0"),
            kind: String::from("function"),
            function: Function {
                name: Some(String::from("f")),
                arguments: String::from("{\"x\":1}"),
            },
        }])
    );
}

#[tokio::test]
async fn json_mode_keeps_a_null_function_name() {
    let found = found(
        r#"{"response":"{\"name\":null,\"arguments\":1}"}"#,
        &json_mode(),
    )
    .await;
    let calls = found.message.tool_calls.unwrap();
    assert_eq!(calls[0].function.name, None);
    assert_eq!(calls[0].function.arguments, "1");
    assert_eq!(calls[0].id, "call_id-0");
}

#[tokio::test]
async fn json_mode_without_a_name_stays_text() {
    let found = found(r#"{"response":"{\"arguments\":1}"}"#, &json_mode()).await;
    assert_eq!(found.message.content.as_deref(), Some("{\"arguments\":1}"));
    assert_eq!(found.finish_reason, "stop");
}

#[tokio::test]
async fn json_mode_falls_back_to_the_reasoning_split_for_invalid_json() {
    let found = found(r#"{"response":"<think>why</think>answer"}"#, &json_mode()).await;
    assert_eq!(found.message.content.as_deref(), Some("answer"));
    assert_eq!(found.message.reasoning_content.as_deref(), Some("why"));
}

#[tokio::test]
async fn json_mode_keeps_the_thinking_field() {
    let found = found(r#"{"response":"{\"a\":1}","thinking":"why"}"#, &json_mode()).await;
    assert_eq!(found.message.content.as_deref(), Some("{\"a\":1}"));
    assert_eq!(found.message.reasoning_content.as_deref(), Some("why"));
}

#[tokio::test]
async fn json_mode_drops_an_empty_thinking_field() {
    let found = found(r#"{"response":"{\"a\":1}","thinking":""}"#, &json_mode()).await;
    assert_eq!(found.message.content.as_deref(), Some("{\"a\":1}"));
    assert_eq!(found.message.reasoning_content.as_deref(), None);
}

#[tokio::test]
async fn a_wrong_typed_response_is_a_decode_error() {
    let error = failed(r#"{"response":5}"#, &nothing()).await;
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_wrong_typed_thinking_is_a_decode_error() {
    let error = failed(r#"{"response":"x","thinking":5}"#, &nothing()).await;
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::non_empty("[1, 2]")]
#[case::empty("[]")]
#[tokio::test]
async fn a_non_object_body_is_a_decode_error(#[case] body: &str) {
    let error = failed(body, &nothing()).await;
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_body_that_is_not_json_is_a_decode_error() {
    let error = failed("not json", &nothing()).await;
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn a_wrong_typed_prompt_is_a_parameter_error() {
    let error = failed(r#"{"response":"x"}"#, &request_data(r#"{"prompt":5}"#)).await;
    assert_eq!(error.status(), Some(400));
}
