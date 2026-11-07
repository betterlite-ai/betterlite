#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;

use crate::pangea::error::PangeaGuardrailError;
use crate::pangea::hooks::{GuardrailResponse, RequestData};
use crate::pangea::tests::support::{
    GUARDRAIL_NAME, Recorder, data_with_messages, guardrail, input_object, payload, serve, user,
};

const ALLOWED: &str = r#"{"result":{"blocked":false,"transformed":false}}"#;
const TRANSFORMED: &str = r#"{"result":{"blocked":false,"transformed":true,"output":{"messages":[{"role":"user","content":"<US_SSN>"}]}}}"#;
const PROMPT: &str = "Hi, my SSN is 078-05-1120";

fn completion_data() -> RequestData {
    RequestData {
        messages: None,
        prompt: Some(GuardrailValue::Text(PROMPT.to_owned())),
        tools: None,
    }
}

#[tokio::test]
async fn text_completion_pre_call_wraps_the_prompt_in_one_user_message() {
    let server = serve(StatusCode::OK, ALLOWED.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = completion_data();
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_pre_call_hook(&mut data, "text_completion", &mut recorder)
        .await
        .unwrap();
    assert!(!transformed);
    let captured = server.captured();
    assert_eq!(captured.path, "/v1beta/guard");
    assert_eq!(
        captured.headers.get("authorization"),
        Some(&"Bearer test-key".to_owned())
    );
    let sent = payload(&captured);
    assert_eq!(sent.event_type, "input");
    assert!(!sent.debug);
    assert_eq!(sent.input.messages, Some(vec![user(PROMPT)]));
    assert_eq!(sent.input.tools, None);
    assert!(!input_object(&captured).contains_key("choices"));
    assert_eq!(recorder.names, [GUARDRAIL_NAME]);
}

#[tokio::test]
async fn text_completion_pre_call_updates_the_prompt_from_the_transformed_message() {
    let server = serve(StatusCode::OK, TRANSFORMED.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = completion_data();
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_pre_call_hook(&mut data, "atext_completion", &mut recorder)
        .await
        .unwrap();
    assert!(transformed);
    assert_eq!(
        data.prompt,
        Some(GuardrailValue::Text("<US_SSN>".to_owned()))
    );
    assert_eq!(data.messages, None);
}

#[tokio::test]
async fn text_completion_pre_call_needs_a_prompt() {
    let instance = guardrail("http://127.0.0.1:1");
    let mut data = data_with_messages(Vec::new());
    let mut recorder = Recorder::default();
    let error = instance
        .async_pre_call_hook(&mut data, "text_completion", &mut recorder)
        .await
        .expect_err("no prompt means no call");
    assert!(matches!(
        error,
        PangeaGuardrailError::MissingData { field: "prompt" }
    ));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn text_completion_pre_call_rejects_a_transformed_answer_without_exactly_one_message() {
    let two_messages = r#"{"result":{"transformed":true,"output":{"messages":[{"role":"user","content":"a"},{"role":"user","content":"b"}]}}}"#;
    let server = serve(StatusCode::OK, two_messages.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = completion_data();
    let mut recorder = Recorder::default();
    let error = instance
        .async_pre_call_hook(&mut data, "text_completion", &mut recorder)
        .await
        .expect_err("two messages cannot become one prompt");
    assert!(matches!(error, PangeaGuardrailError::InvalidTextCompletion));
    assert_eq!(data.prompt, Some(GuardrailValue::Text(PROMPT.to_owned())));
}

#[tokio::test]
async fn text_completion_post_call_sends_the_prompt_as_the_input_message() {
    let server = serve(StatusCode::OK, ALLOWED.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = completion_data();
    let mut response = GuardrailResponse::TextCompletion {
        choices: Vec::new(),
    };
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_post_call_success_hook(&mut data, &mut response, &mut recorder)
        .await
        .unwrap();
    assert!(!transformed);
    let sent = payload(&server.captured());
    assert_eq!(sent.event_type, "output");
    assert_eq!(sent.input.messages, Some(vec![user(PROMPT)]));
    assert_eq!(sent.input.choices, Some(Vec::new()));
    assert_eq!(recorder.names, [GUARDRAIL_NAME]);
}

#[tokio::test]
async fn text_completion_post_call_replaces_the_choices() {
    let answer =
        r#"{"result":{"transformed":true,"output":{"choices":[{"text":"<US_SSN>","index":0}]}}}"#;
    let server = serve(StatusCode::OK, answer.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = completion_data();
    let choice = GuardrailValue::Object(BTreeMap::from([(
        "text".to_owned(),
        GuardrailValue::Text("078-05-1120".to_owned()),
    )]));
    let mut response = GuardrailResponse::TextCompletion {
        choices: vec![choice.clone()],
    };
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_post_call_success_hook(&mut data, &mut response, &mut recorder)
        .await
        .unwrap();
    assert!(transformed);
    let GuardrailResponse::TextCompletion { choices } = response else {
        panic!("the response kind stays a text completion");
    };
    assert_eq!(choices.len(), 1);
    assert_ne!(choices, vec![choice.clone()]);
    let sent = payload(&server.captured());
    assert_eq!(sent.input.choices, Some(vec![choice]));
}
