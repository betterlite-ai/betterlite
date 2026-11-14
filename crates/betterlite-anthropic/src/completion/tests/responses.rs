#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_openai::chat::response::ChatResponse;
use chrono::{DateTime, Utc};

use crate::completion::error::CompletionError;
use crate::completion::prompt::{CustomPromptEntry, PromptSource};
use crate::completion::response::{ResponseInput, resolve_tokenizer, transform_response};

use super::{
    USER_PROMPT, blank_response, clock, default_encoding, factory, templates, tiktoken,
    user_messages,
};

const HELLO_WORLD: &str = r#"{"completion":"hello world","stop_reason":"stop_sequence"}"#;
const EMPTY_COMPLETION: &str = r#"{"completion":"","stop_reason":null}"#;

async fn answer(
    dict: &BTreeMap<String, CustomPromptEntry>,
    status_code: u16,
    body: &str,
    response: ChatResponse,
) -> Result<ChatResponse, CompletionError> {
    let prompts = factory();
    let source = PromptSource {
        custom_prompt_dict: dict,
        prompts: &prompts,
        now: clock(),
    };
    let messages = user_messages();
    let input = ResponseInput {
        model: "claude-2",
        messages: &messages,
        status_code,
        body: body.as_bytes(),
    };
    transform_response(&source, &input, response, &tiktoken()).await
}

fn count(text: &str) -> u64 {
    u64::try_from(tiktoken().encode(text).unwrap().len()).unwrap()
}

fn failure(result: Result<ChatResponse, CompletionError>) -> (u16, String) {
    let Err(CompletionError::Upstream(error)) = result else {
        panic!("expected an upstream error, got {result:?}");
    };
    (error.status_code, error.message)
}

pub(super) async fn completion_replaces_content() {
    let none = BTreeMap::new();
    let done = answer(&none, 200, HELLO_WORLD, blank_response(Some("old"))).await;
    let done = done.unwrap();
    let choice = &done.choices[0];
    assert_eq!(choice.message.content.as_deref(), Some("hello world"));
    assert_eq!(choice.finish_reason.as_deref(), Some("stop_sequence"));
    assert_eq!(done.id, "chatcmpl-kept");
    assert_eq!(done.model, "claude-2");
    assert_eq!(done.created, u64::try_from(clock().timestamp()).unwrap());
    let prompt_tokens = count(USER_PROMPT);
    assert_eq!(done.usage.prompt_tokens, prompt_tokens);
    assert_eq!(done.usage.completion_tokens, 2);
    assert_eq!(
        done.usage.total_tokens,
        prompt_tokens.checked_add(2).unwrap()
    );
}

pub(super) async fn empty_completion_keeps_content() {
    let none = BTreeMap::new();
    let kept = answer(
        &none,
        200,
        EMPTY_COMPLETION,
        blank_response(Some("hello world")),
    )
    .await;
    let kept = kept.unwrap();
    assert_eq!(
        kept.choices[0].message.content.as_deref(),
        Some("hello world")
    );
    assert_eq!(kept.choices[0].finish_reason, None);
    assert_eq!(kept.usage.completion_tokens, 2);
    let unset = answer(&none, 200, EMPTY_COMPLETION, blank_response(None)).await;
    let unset = unset.unwrap();
    assert_eq!(unset.choices[0].message.content, None);
    assert_eq!(unset.usage.completion_tokens, 0);
}

pub(super) async fn prompt_is_rebuilt() {
    let custom = answer(&templates(), 200, HELLO_WORLD, blank_response(None)).await;
    assert_eq!(custom.unwrap().usage.prompt_tokens, count("<U:hello|>"));
}

// a clock before 1970 gives created 0 where Python's int(time.time()) would be negative
pub(super) async fn created_clamps_a_clock_before_the_epoch() {
    let prompts = factory();
    let none = BTreeMap::new();
    let source = PromptSource {
        custom_prompt_dict: &none,
        prompts: &prompts,
        now: DateTime::<Utc>::MIN_UTC,
    };
    let messages = user_messages();
    let input = ResponseInput {
        model: "claude-2",
        messages: &messages,
        status_code: 200,
        body: HELLO_WORLD.as_bytes(),
    };
    let done = transform_response(&source, &input, blank_response(None), &tiktoken()).await;
    assert_eq!(done.unwrap().created, 0);
}

pub(super) async fn failures() {
    let none = BTreeMap::new();
    let html = "<html>bad gateway</html>";
    let broken = answer(&none, 502, html, blank_response(None)).await;
    assert_eq!(failure(broken), (502, html.to_owned()));
    let text = r#"{"type":"error","error":"overloaded"}"#;
    let text = answer(&none, 529, text, blank_response(None)).await;
    assert_eq!(failure(text), (529, "overloaded".to_owned()));
    let object = r#"{"error":{"message":"bad"}}"#;
    let object = answer(&none, 400, object, blank_response(None)).await;
    assert_eq!(failure(object), (400, r#"{"message":"bad"}"#.to_owned()));
    let missing = answer(&none, 200, "{}", blank_response(None)).await;
    assert!(matches!(missing, Err(CompletionError::MissingField { .. })));
    let mut empty = blank_response(None);
    empty.choices.clear();
    let choiceless = answer(&none, 200, HELLO_WORLD, empty).await;
    assert!(matches!(choiceless, Err(CompletionError::NoChoices)));
    let no_body = answer(&none, 200, "", blank_response(None)).await;
    assert_eq!(failure(no_body), (200, String::new()));
}

pub(super) async fn tokenizer_choice() {
    let explicit = resolve_tokenizer(Some(tiktoken()), &default_encoding()).await;
    let expected = [15339_u32, 1917];
    assert_eq!(explicit.unwrap().encode("hello world").unwrap(), expected);
    let fallback = resolve_tokenizer(None, &default_encoding()).await;
    assert_ne!(fallback.unwrap().encode("hello world").unwrap(), expected);
}
