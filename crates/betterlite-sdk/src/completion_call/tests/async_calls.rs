use std::sync::Arc;
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::support::{FakeCallback, Harness, chat_body, dig, json, request};
use super::{USER, answer_choices, body_text};
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::error::{class_name, class_status};
use crate::call_wrapper::target::CallTarget as _;
use crate::completion_call::error::CompletionCallError;

#[tokio::test]
async fn async_mock_delay_sleeps_through_the_injected_sleeper() {
    let harness = Harness::new();
    let target = harness.async_target();
    let call = request(&format!(
        r#"{{"model":"gpt-4o","messages":[{USER}],"mock_response":"hi","mock_delay":0.5}}"#
    ));
    let outcome = target.call(call, None).await.unwrap();
    assert_eq!(harness.sleeper.slept(), vec![Duration::from_millis(500)]);
    let Some(answer) = body_text(&outcome) else {
        panic!("the delayed mock answers");
    };
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("hi")))
    );
}

#[tokio::test]
async fn async_text_completion_reply_becomes_a_chat_reply() {
    let harness = Harness::new();
    let target = harness.async_target();
    let reply = json(
        r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"gpt-3.5-turbo-instruct","choices":[{"text":"hello","index":0,"finish_reason":"stop"}]}"#,
    );
    let converted = target
        .convert_text_reply(Some("text-completion-openai"), CallOutcome::Response(reply))
        .unwrap();
    let Some(answer) = body_text(&converted) else {
        panic!("the converted answer is a response");
    };
    assert_eq!(
        dig(answer, &["object"]),
        Some(&Json::Text(String::from("chat.completion")))
    );
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("hello")))
    );
    assert_eq!(
        dig(answer, &["model"]),
        Some(&Json::Text(String::from("gpt-3.5-turbo-instruct")))
    );
}

#[tokio::test]
async fn async_other_providers_keep_their_reply() {
    let harness = Harness::new();
    let target = harness.async_target();
    let reply =
        json(r#"{"id":"cmpl-1","object":"text_completion","created":1,"model":"m","choices":[]}"#);
    let outcome = target
        .convert_text_reply(Some("openai"), CallOutcome::Response(reply))
        .unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("the reply is a response");
    };
    assert_eq!(
        dig(answer, &["object"]),
        Some(&Json::Text(String::from("text_completion")))
    );
}

#[tokio::test]
async fn async_agentic_loop_replaces_the_response() {
    let mut harness = Harness::new();
    harness.callbacks.push(Arc::new(FakeCallback {
        reply: chat_body("looped"),
    }));
    let target = harness.async_target();
    let call = request(&format!(r#"{{"model":"gpt-4o","messages":[{USER}]}}"#));
    let outcome = target.call(call, None).await.unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("the loop answers a response");
    };
    assert_eq!(
        dig(answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-agentic")))
    );
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("looped")))
    );
}

#[tokio::test]
async fn async_error_defaults_the_provider_to_openai() {
    let harness = Harness::new();
    let target = harness.async_target();
    let call = request(&format!(
        r#"{{"model":"missing-model","messages":[{USER}]}}"#
    ));
    let error = target.call(call, None).await.unwrap_err();
    let ErrorClass::BadRequest(details) = &error.class else {
        panic!("a missing deployment is a bad request");
    };
    assert_eq!(details.provider.as_deref(), Some("openai"));
}

#[test]
fn async_fallbacks_with_no_answer_uses_litellms_message() {
    assert_eq!(
        CompletionCallError::NoFallbackResponse.to_string(),
        "No response from fallbacks. Got none. Turn on `litellm.set_verbose=True` to see more details."
    );
}

#[tokio::test]
async fn async_text_provider_keeps_a_chat_shaped_answer() {
    let harness = Harness::new();
    let target = harness.async_target();
    let outcome = target
        .convert_text_reply(
            Some("text-completion-openai"),
            CallOutcome::Response(json(&chat_body("kept"))),
        )
        .unwrap();
    let Some(answer) = body_text(&outcome) else {
        panic!("the provider's chat reply stays a response");
    };
    assert_eq!(
        dig(answer, &["id"]),
        Some(&Json::Text(String::from("chatcmpl-agentic")))
    );
    let choices = answer_choices(answer);
    assert_eq!(
        dig(&choices[0], &["message", "content"]),
        Some(&Json::Text(String::from("kept")))
    );
}

#[tokio::test]
async fn async_mock_timeout_is_a_timeout_error() {
    let harness = Harness::new();
    let target = harness.async_target();
    let call = request(&format!(
        r#"{{"model":"gpt-4o","messages":[{USER}],"mock_timeout":true,"timeout":0}}"#
    ));
    let error = target.call(call, None).await.unwrap_err();
    assert_eq!(class_name(&error.class), "Timeout");
    assert_eq!(class_status(&error.class), Some(408));
    assert!(error.to_string().contains("Timeout Error:"), "{error}");
}
