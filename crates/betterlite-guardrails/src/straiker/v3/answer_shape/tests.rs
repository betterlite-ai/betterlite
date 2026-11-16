#![cfg(test)]

use rstest::rstest;

use super::{AnswerContext, Response, answer};
use crate::straiker::v3::fixtures::{Routes, Translator, as_json, object, parsed};

enum Kind {
    TextCompletion,
    Chat,
    Other,
}

const MESSAGES_ROUTE: &str =
    r#"{"litellm_metadata":{"user_api_key_request_route":"/v1/messages"}}"#;
const CHAT_ROUTE: &str =
    r#"{"litellm_metadata":{"user_api_key_request_route":"/v1/chat/completions"}}"#;
const TEXT_COMPLETION: &str = r#"{"id":"cmpl-1","created":5,"model":"davinci","choices":[{"finish_reason":"stop","text":"hi"}]}"#;
const CHAT_ANSWER: &str = r#"{"id":"chatcmpl-1","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":"hi"}}]}"#;

#[rstest]
#[case::text_completion_becomes_chat(Kind::TextCompletion, TEXT_COMPLETION, CHAT_ROUTE, None, r#"{"id":"cmpl-1","object":"chat.completion","created":5,"model":"davinci","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"hi"}}]}"#)]
#[case::chat_stays_chat_on_chat_route(Kind::Chat, CHAT_ANSWER, CHAT_ROUTE, None, CHAT_ANSWER)]
#[case::chat_stays_chat_without_a_route(Kind::Chat, CHAT_ANSWER, r"{}", None, CHAT_ANSWER)]
#[case::chat_becomes_messages_answer(
    Kind::Chat,
    CHAT_ANSWER,
    MESSAGES_ROUTE,
    Some("fallback"),
    r#"{"id":"chatcmpl-1","type":"message","model":"gpt-4o"}"#
)]
#[case::messages_answer_takes_proxy_model(
    Kind::Chat,
    r#"{"id":"c1"}"#,
    MESSAGES_ROUTE,
    Some("fallback"),
    r#"{"id":"c1","type":"message","model":"fallback"}"#
)]
#[case::empty_chat_model_takes_proxy_model(
    Kind::Chat,
    r#"{"id":"c1","model":""}"#,
    MESSAGES_ROUTE,
    Some("fallback"),
    r#"{"id":"c1","type":"message","model":"fallback"}"#
)]
#[case::messages_answer_without_any_model(
    Kind::Chat,
    r#"{"id":"c1"}"#,
    MESSAGES_ROUTE,
    None,
    r#"{"id":"c1","type":"message","model":null}"#
)]
#[case::dict_is_relayed(
    Kind::Other,
    r#"{"output":[{"type":"message"}]}"#,
    MESSAGES_ROUTE,
    None,
    r#"{"output":[{"type":"message"}]}"#
)]
#[case::text_is_no_answer(Kind::Other, r#""done""#, CHAT_ROUTE, None, "null")]
#[case::null_is_no_answer(Kind::Other, "null", CHAT_ROUTE, None, "null")]
fn shapes_answer(
    #[case] kind: Kind,
    #[case] response: &str,
    #[case] request_data: &str,
    #[case] model: Option<&str>,
    #[case] expected: &str,
) {
    let request_data = object(request_data);
    let context = AnswerContext {
        request_data: &request_data,
        routes: &Routes,
        translator: &Translator,
        model,
    };
    let response = match kind {
        Kind::TextCompletion => Response::TextCompletion(serde_json::from_str(response).unwrap()),
        Kind::Chat => Response::Chat(object(response)),
        Kind::Other => Response::Other(parsed(response)),
    };
    assert_eq!(as_json(&answer(&response, &context)), parsed(expected));
}
