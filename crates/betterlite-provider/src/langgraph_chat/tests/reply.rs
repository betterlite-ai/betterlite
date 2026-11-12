use async_trait::async_trait;
use betterlite_openai::error::ProviderError;
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::error::CountError;
use rstest::rstest;

use super::{WordCounter, run_reply, warn_log};
use crate::langgraph_chat::records::{LangGraphRole, Usage};
use crate::langgraph_chat::reply::{LangGraphChoice, TokenCounter};

const NO_TEXT_WARNING: &str = "Failed to calculate token usage: the reply has no text to count";
const RAW_WARNING: &str = "Could not extract content from LangGraph response, returning raw";

struct FailingCounter;

#[async_trait]
impl TokenCounter for FailingCounter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Err(CountError::Overflow)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(1)
    }
}

struct HugeCounter;

#[async_trait]
impl TokenCounter for HugeCounter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(u64::MAX)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(1)
    }
}

struct CompletionFailsCounter;

#[async_trait]
impl TokenCounter for CompletionFailsCounter {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(1)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Err(CountError::Boundary)
    }
}

fn content_of(body: &str) -> Option<String> {
    let reply = run_reply(body, 200, &WordCounter).unwrap();
    assert_eq!(reply.choices.len(), 1);
    reply.choices[0].content.clone()
}

#[test]
fn the_reply_is_one_stop_assistant_choice_for_the_requested_model() {
    let reply = run_reply(
        r#"{"messages":[{"type":"human","content":"hi"},{"type":"ai","content":"first"},{"type":"ai","content":"two words"}]}"#,
        200,
        &WordCounter,
    )
    .unwrap();
    assert_eq!(reply.model, "langgraph/agent");
    assert_eq!(
        reply.choices,
        vec![LangGraphChoice {
            index: 0,
            finish_reason: "stop",
            role: LangGraphRole::Assistant,
            content: Some(String::from("two words")),
        }]
    );
    assert_eq!(
        reply.usage,
        Some(Usage {
            prompt_tokens: 10,
            completion_tokens: 2,
            total_tokens: 12,
        })
    );
}

#[rstest]
#[case::last_ai_message(
    r#"{"messages":[{"type":"ai","content":"old"},{"type":"human","content":"q"},{"type":"ai","content":"new"},{"type":"human","content":"later"}]}"#,
    "new"
)]
#[case::assistant_role(
    r#"{"messages":[{"type":"ai","content":"old"},{"role":"assistant","content":"new"}]}"#,
    "new"
)]
#[case::messages_before_values(
    r#"{"messages":[{"type":"ai","content":"top"}],"values":{"messages":[{"type":"ai","content":"inner"}]}}"#,
    "top"
)]
#[case::values_when_no_ai_message(
    r#"{"messages":[{"type":"human","content":"q"}],"values":{"messages":[{"type":"ai","content":"old"},{"type":"ai","content":"inner"}]}}"#,
    "inner"
)]
#[case::values_without_top_messages(
    r#"{"values":{"messages":[{"type":"ai","content":"inner"}]}}"#,
    "inner"
)]
#[case::missing_content_is_empty_and_stops_the_search(
    r#"{"messages":[{"type":"ai","content":"older"},{"type":"ai"}]}"#,
    ""
)]
#[case::empty_content_stops_the_search(
    r#"{"messages":[{"type":"ai","content":"older"},{"type":"ai","content":""}]}"#,
    ""
)]
#[case::values_missing_content(
    r#"{"values":{"messages":[{"type":"ai","content":"older"},{"type":"ai"}]}}"#,
    ""
)]
fn the_reply_text_comes_from_the_last_ai_message(#[case] body: &str, #[case] expected: &str) {
    assert_eq!(content_of(body).as_deref(), Some(expected));
}

#[rstest]
#[case::values_ignore_the_assistant_role(
    r#"{"values":{"messages":[{"role":"assistant","content":"y"}]}}"#
)]
#[case::no_ai_message(r#"{"messages":[{"type":"human","content":"x"}]}"#)]
#[case::empty_object("{}")]
#[case::messages_not_a_list(r#"{"messages":"x"}"#)]
#[case::values_not_an_object(r#"{"values":[1]}"#)]
#[case::values_messages_not_a_list(r#"{"values":{"messages":{"type":"ai"}}}"#)]
fn without_an_ai_message_the_reply_text_is_the_raw_json(#[case] body: &str) {
    let (content, log) = warn_log(|| content_of(body));
    let value: Json = serde_json::from_str(body).unwrap();
    let expected = serde_json::to_string(&value).unwrap();
    assert_eq!(
        content.unwrap().replace(", ", ",").replace(": ", ":"),
        expected
    );
    assert!(log.contains(RAW_WARNING), "{log}");
}

#[test]
fn the_raw_fallback_is_written_like_python_json_dumps() {
    let (content, _log) = warn_log(|| content_of(r#"{"a":[1,2],"t":"\u00e9"}"#));
    assert_eq!(content.unwrap(), r#"{"a": [1, 2], "t": "\u00e9"}"#);
}

#[test]
fn a_found_message_logs_no_raw_fallback_warning() {
    let (_content, log) =
        warn_log(|| content_of(r#"{"messages":[{"type":"ai","content":"x y"}]}"#));
    assert!(!log.contains(RAW_WARNING), "{log}");
    assert!(!log.contains("Failed to calculate"), "{log}");
}

#[test]
fn a_null_content_leaves_the_text_and_usage_unset_and_logs_a_warning() {
    let (reply, log) = warn_log(|| {
        run_reply(
            r#"{"messages":[{"type":"ai","content":null}]}"#,
            200,
            &WordCounter,
        )
        .unwrap()
    });
    assert_eq!(reply.choices[0].content, None);
    assert_eq!(reply.usage, None);
    assert!(log.contains(NO_TEXT_WARNING), "{log}");
}

#[rstest]
#[case::prompt_count_fails(&FailingCounter, "Failed to calculate token usage: the tokens cannot be counted: the token count does not fit in a machine word")]
#[case::completion_count_fails(&CompletionFailsCounter, "Failed to calculate token usage: the tokens cannot be counted: a text boundary does not fall on a character boundary")]
#[case::total_overflows(&HugeCounter, "Failed to calculate token usage: the token usage overflows")]
fn a_usage_failure_keeps_the_reply_and_logs_a_warning(
    #[case] counter: &dyn TokenCounter,
    #[case] warning: &str,
) {
    let (reply, log) = warn_log(|| {
        run_reply(
            r#"{"messages":[{"type":"ai","content":"hello there"}]}"#,
            200,
            counter,
        )
        .unwrap()
    });
    assert_eq!(reply.choices[0].content.as_deref(), Some("hello there"));
    assert_eq!(reply.usage, None);
    assert!(log.contains(warning), "{log}");
}

#[rstest]
#[case::not_json(
    "not json",
    502,
    "Error processing response: the LangGraph reply is not JSON: "
)]
#[case::empty_body(
    "",
    500,
    "Error processing response: the LangGraph reply is not JSON: "
)]
#[case::list_body(
    "[1]",
    200,
    "Error processing response: the LangGraph reply is not a JSON object"
)]
#[case::text_body(
    "\"x\"",
    418,
    "Error processing response: the LangGraph reply is not a JSON object"
)]
#[case::number_content(
    r#"{"messages":[{"type":"ai","content":5}]}"#,
    503,
    "Error processing response: the content of the LangGraph reply message is not text"
)]
#[case::list_content(
    r#"{"values":{"messages":[{"type":"ai","content":["x"]}]}}"#,
    504,
    "Error processing response: the content of the LangGraph reply message is not text"
)]
fn a_failure_is_wrapped_with_the_http_status(
    #[case] body: &str,
    #[case] status: u16,
    #[case] message: &str,
) {
    let error = run_reply(body, status, &WordCounter).unwrap_err();
    assert_eq!(error.status_code, status);
    assert!(error.message.starts_with(message), "{}", error.message);
}

#[test]
fn the_wrapped_failure_logs_an_error_line() {
    let (error, log) = warn_log(|| run_reply("nope", 502, &WordCounter).unwrap_err());
    assert_eq!(error.status_code, 502);
    assert!(
        log.contains("Error processing LangGraph response: the LangGraph reply is not JSON"),
        "{log}"
    );
}

#[test]
fn a_wrapped_failure_converts_into_the_provider_error_with_its_status() {
    let error = run_reply("nope", 502, &WordCounter).unwrap_err();
    let message = error.message.clone();
    let provider = ProviderError::from(error);
    assert_eq!(provider.status_code, 502);
    assert_eq!(provider.message, message);
    assert!(!provider.status_code_is_synthesized);
    assert_eq!(provider.headers, None);
}
