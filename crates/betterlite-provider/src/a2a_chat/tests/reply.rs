use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_openai::error::ProviderError;
use betterlite_prompts::message_content::Message;
use betterlite_tokenizer::count::error::CountError;
use rstest::rstest;
use tokio::runtime::{Builder, Runtime};

use crate::a2a_chat::error::{A2aError, get_error_class};
use crate::a2a_chat::records::{A2aReply, Usage};
use crate::a2a_chat::reply::{ReplyInput, TokenCounter, transform_response};
use crate::ollama_chat::ids::IdSource;

use super::support::{CountedIds, header_map, messages};

fn runtime() -> Runtime {
    Builder::new_current_thread().build().unwrap()
}

struct FixedCounter {
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[async_trait]
impl TokenCounter for FixedCounter {
    async fn count_messages(&self, model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        assert_eq!(model, "gpt-3.5-turbo");
        Ok(self.prompt_tokens)
    }

    async fn count_response_tokens(&self, model: &str, _text: &str) -> Result<u64, CountError> {
        assert_eq!(model, "gpt-3.5-turbo");
        Ok(self.completion_tokens)
    }
}

struct PromptFails;

#[async_trait]
impl TokenCounter for PromptFails {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Err(CountError::Overflow)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Ok(1)
    }
}

struct CompletionFails;

#[async_trait]
impl TokenCounter for CompletionFails {
    async fn count_messages(&self, _model: &str, _messages: &[Message]) -> Result<u64, CountError> {
        Ok(1)
    }

    async fn count_response_tokens(&self, _model: &str, _text: &str) -> Result<u64, CountError> {
        Err(CountError::Boundary)
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

fn reply(
    body: &str,
    status: u16,
    ids: &dyn IdSource,
    counter: &dyn TokenCounter,
) -> Result<A2aReply, A2aError> {
    let prompt = messages(r#"[{"role":"user","content":"hi"}]"#);
    let headers = header_map(&[("x", "y")]);
    let input = ReplyInput {
        model: "a2a/my-agent",
        status,
        headers: &headers,
        body: body.as_bytes(),
        messages: &prompt,
    };
    runtime().block_on(transform_response(&input, ids, counter))
}

#[test]
fn get_error_class_carries_the_status_message_and_headers() {
    let headers = header_map(&[("x", "y")]);
    let error = get_error_class("boom", 418, headers.clone());
    assert_eq!(error.status_code, 418);
    assert_eq!(error.message, "boom");
    assert_eq!(error.headers, headers);
    assert_eq!(error.to_string(), "boom");
}

#[test]
fn an_a2a_error_becomes_a_provider_error() {
    let error = get_error_class("boom", 418, header_map(&[("x", "y")]));
    let provider = ProviderError::from(error);
    assert_eq!(provider.status_code, 418);
    assert_eq!(provider.message, "boom");
    assert_eq!(
        provider.headers,
        Some(vec![(String::from("x"), String::from("y"))])
    );
}

#[test]
fn a_provider_error_from_an_a2a_error_has_no_headers_when_empty() {
    let provider = ProviderError::from(get_error_class("boom", 500, BTreeMap::new()));
    assert_eq!(provider.status_code, 500);
    assert_eq!(provider.headers, None);
}

#[test]
fn a_text_reply_is_one_stop_choice_with_the_requested_model() {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 2,
        completion_tokens: 3,
    };
    let body = r#"{"jsonrpc":"2.0","id":"r","result":{"parts":[{"kind":"text","text":"hello"}]}}"#;
    let response = reply(body, 200, &ids, &counter).unwrap();
    assert_eq!(response.id, "r");
    assert_eq!(response.model, "a2a/my-agent");
    assert_eq!(
        response.usage,
        Some(Usage {
            prompt_tokens: 2,
            completion_tokens: 3,
            total_tokens: 5,
        })
    );
    assert_eq!(response.choices.len(), 1);
    assert_eq!(response.choices[0].index, 0);
    assert_eq!(response.choices[0].finish_reason, "stop");
    assert_eq!(response.choices[0].message.role, "assistant");
    assert_eq!(response.choices[0].message.content, "hello");
}

#[test]
fn zero_token_counts_stay_a_usage_of_zeros() {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let body = r#"{"id":"r","result":{"parts":[{"kind":"text","text":"x"}]}}"#;
    let response = reply(body, 200, &ids, &counter).unwrap();
    assert_eq!(
        response.usage,
        Some(Usage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
        })
    );
}

#[rstest]
#[case::absent(r#"{"result":{"parts":[]}}"#)]
#[case::null(r#"{"id":null,"result":{"parts":[]}}"#)]
fn a_missing_reply_id_gets_the_next_id(#[case] body: &str) {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let response = reply(body, 200, &ids, &counter).unwrap();
    assert_eq!(response.id, "id-0");
}

#[test]
fn an_empty_string_reply_id_is_kept() {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let response = reply(r#"{"id":"","result":{"parts":[]}}"#, 200, &ids, &counter).unwrap();
    assert_eq!(response.id, "");
}

#[rstest]
#[case::number(r#"{"id":7,"result":{"parts":[]}}"#)]
#[case::flag(r#"{"id":true,"result":{"parts":[]}}"#)]
#[case::list(r#"{"id":[],"result":{"parts":[]}}"#)]
#[case::object(r#"{"id":{},"result":{"parts":[]}}"#)]
fn a_non_string_reply_id_names_it(#[case] body: &str) {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let error = reply(body, 200, &ids, &counter).unwrap_err();
    assert_eq!(error.status_code, 200);
    assert_eq!(error.message, "the A2A reply id is not a string");
    assert_eq!(error.headers, header_map(&[("x", "y")]));
}

#[test]
fn the_error_key_carries_the_reply_status_and_message() {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let body = r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"boom"}}"#;
    let error = reply(body, 200, &ids, &counter).unwrap_err();
    assert_eq!(error.status_code, 200);
    assert_eq!(error.message, "A2A error: boom");
    assert_eq!(error.headers, header_map(&[("x", "y")]));
}

#[rstest]
#[case::empty_object(r#"{"error":{}}"#, 503)]
#[case::non_object(r#"{"error":"boom"}"#, 200)]
#[case::null(r#"{"error":null}"#, 200)]
fn an_error_without_a_message_says_unknown(#[case] body: &str, #[case] status: u16) {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let error = reply(body, status, &ids, &counter).unwrap_err();
    assert_eq!(error.status_code, status);
    assert_eq!(error.message, "A2A error: Unknown error");
}

#[test]
fn a_body_that_is_not_json_is_the_parse_error() {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let error = reply("not json", 418, &ids, &counter).unwrap_err();
    assert_eq!(error.status_code, 418);
    assert!(error.message.starts_with("Failed to parse A2A response: "));
    assert_eq!(error.headers, header_map(&[("x", "y")]));
}

#[rstest]
#[case::list("[1,2]")]
#[case::text(r#""text""#)]
#[case::number("7")]
#[case::null("null")]
fn a_body_that_is_not_an_object_names_the_reply(#[case] body: &str) {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let error = reply(body, 200, &ids, &counter).unwrap_err();
    assert_eq!(error.status_code, 200);
    assert_eq!(error.message, "the A2A reply is not a JSON object");
}

#[rstest]
#[case::nested_message(
    r#"{"result":{"message":{"parts":[{"kind":"text","text":"nested"}]}}}"#,
    "nested"
)]
#[case::artifacts(
    r#"{"result":{"artifacts":[{"parts":[{"kind":"text","text":"first"},{"parts":[{"kind":"text","text":"second"}]}]}]}}"#,
    "first second"
)]
#[case::status_message(
    r#"{"result":{"status":{"message":{"parts":[{"kind":"text","text":"status"}]}}}}"#,
    "status"
)]
fn text_is_read_from_the_declared_result_shapes(#[case] body: &str, #[case] expected: &str) {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let response = reply(body, 200, &ids, &counter).unwrap();
    assert_eq!(response.choices[0].message.content, expected);
}

#[test]
fn an_unreadable_result_becomes_a_reply_error() {
    let ids = CountedIds::new();
    let counter = FixedCounter {
        prompt_tokens: 0,
        completion_tokens: 0,
    };
    let body = r#"{"result":{"parts":[{"kind":"text","text":7}]}}"#;
    let error = reply(body, 200, &ids, &counter).unwrap_err();
    assert_eq!(error.status_code, 200);
    assert!(
        error
            .message
            .starts_with("the A2A reply text cannot be extracted: ")
    );
}

#[test]
fn a_failing_prompt_counter_leaves_usage_unset() {
    let ids = CountedIds::new();
    let body = r#"{"id":"r","result":{"parts":[{"kind":"text","text":"x"}]}}"#;
    let response = reply(body, 200, &ids, &PromptFails).unwrap();
    assert_eq!(response.usage, None);
}

#[test]
fn a_failing_completion_counter_leaves_usage_unset() {
    let ids = CountedIds::new();
    let body = r#"{"id":"r","result":{"parts":[{"kind":"text","text":"x"}]}}"#;
    let response = reply(body, 200, &ids, &CompletionFails).unwrap();
    assert_eq!(response.usage, None);
}

#[test]
fn an_overflowing_token_total_leaves_usage_unset() {
    let ids = CountedIds::new();
    let body = r#"{"id":"r","result":{"parts":[{"kind":"text","text":"x"}]}}"#;
    let response = reply(body, 200, &ids, &HugeCounter).unwrap();
    assert_eq!(response.usage, None);
}
