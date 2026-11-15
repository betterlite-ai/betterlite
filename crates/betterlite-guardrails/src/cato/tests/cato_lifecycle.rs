use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use secrecy::SecretString;

use super::support::{
    Answer, PostedBody, Redacted, action, anonymized, assert_header, build, chat_data,
    chat_response, no_action, serve,
};
use super::{array_of, body, field_text, list, object, text};
use crate::cato::error::CatoError;
use crate::cato::inspection::CallerArgs;
use crate::cato::inspection::hooks::{moderation_hook, pre_call_hook};
use crate::cato::inspection::output::fragments::{Step, write_fragment};
use crate::cato::inspection::output::{ResponseKind, post_call_success_hook};
use crate::cato::{CatoGuardrail, WsSsl};

mod streaming;
mod websocket;

const CALLER: CallerArgs<'static> = CallerArgs {
    key_alias: Some("alias"),
    user_email: Some("user@example.test"),
};

fn ws_guardrail(ws_api_base: &str, ws_ssl: Option<WsSsl>, timeout: Duration) -> CatoGuardrail {
    CatoGuardrail {
        guardrail_name: Some("cato".to_owned()),
        api_key: SecretString::from("test-key"),
        api_base: ws_api_base.to_owned(),
        ws_api_base: ws_api_base.to_owned(),
        inspect_embeddings: false,
        ws_ssl,
        timeout,
        client: reqwest::Client::default(),
    }
}

fn first_message(response: &GuardrailValue) -> &BTreeMap<String, GuardrailValue> {
    let GuardrailValue::Object(response) = response else {
        panic!("the response is not an object");
    };
    let Some(GuardrailValue::Array(choices)) = response.get("choices") else {
        panic!("the response has no choices");
    };
    let Some(GuardrailValue::Object(choice)) = choices.first() else {
        panic!("the choice is not an object");
    };
    let Some(GuardrailValue::Object(message)) = choice.get("message") else {
        panic!("the message is not an object");
    };
    message
}

fn argument(message: &BTreeMap<String, GuardrailValue>, index: usize) -> Option<&str> {
    let Some(GuardrailValue::Array(calls)) = message.get("tool_calls") else {
        return None;
    };
    let Some(GuardrailValue::Object(call)) = calls.get(index) else {
        return None;
    };
    let Some(GuardrailValue::Object(function)) = call.get("function") else {
        return None;
    };
    match function.get("arguments") {
        Some(GuardrailValue::Text(arguments)) => Some(arguments),
        _ => None,
    }
}

fn output_items(response: &GuardrailValue) -> &[GuardrailValue] {
    let GuardrailValue::Object(response) = response else {
        panic!("the response is not an object");
    };
    match response.get("output") {
        Some(GuardrailValue::Array(output)) => output,
        _ => panic!("the response has no output"),
    }
}

async fn output_hook(
    answer: Answer,
    response: &mut GuardrailValue,
    kind: ResponseKind,
) -> (Result<(), CatoError>, Vec<PostedBody>) {
    let (base, mut server, captured) = serve(axum::http::StatusCode::OK, answer).await;
    let result = post_call_success_hook(&build(&base), &chat_data(), CALLER, kind, response).await;
    let recorded = captured.lock().expect("captured").clone();
    server.shutdown().await;
    let posted = recorded
        .iter()
        .map(|(_headers, request_body)| serde_json::from_str(request_body).expect("posted body"))
        .collect();
    (result, posted)
}

#[tokio::test]
async fn request_hooks_inspect_the_request_with_their_own_hook_header() {
    let (base, mut server, captured) = serve(axum::http::StatusCode::OK, no_action()).await;
    let mut data = chat_data();
    let original = data.clone();
    pre_call_hook(&build(&base), &mut data, "completion", CALLER)
        .await
        .expect("inspected");
    moderation_hook(
        &build(&base),
        &mut data,
        "completion",
        CallerArgs::default(),
    )
    .await
    .expect("inspected");
    let recorded = captured.lock().expect("captured").clone();
    server.shutdown().await;
    assert_eq!(data, original);
    assert_eq!(recorded.len(), 2);
    let (pre_call, posted) = recorded.first().expect("one request");
    assert_header(pre_call, "x-cato-litellm-hook", "pre_call");
    assert_header(pre_call, "x-cato-call-id", "call-1");
    assert_header(pre_call, "x-cato-user-email", "user@example.test");
    assert_header(pre_call, "x-cato-gateway-key-alias", "alias");
    let (moderation, _body) = recorded.get(1).expect("second request");
    assert_header(moderation, "x-cato-litellm-hook", "moderation");
    assert!(!moderation.contains_key("x-cato-user-email"));
    let posted: PostedBody = serde_json::from_str(posted).expect("posted body");
    assert_eq!(
        posted.messages,
        vec![object(&[
            ("content", text("hello")),
            ("role", text("user"))
        ])]
    );
}

#[tokio::test]
async fn request_hooks_skip_embedding_call_types_unless_inspecting_embeddings() {
    let (base, mut server, captured) = serve(axum::http::StatusCode::OK, no_action()).await;
    let guardrail = build(&base);
    let mut data = body(&[("input", text("a document"))]);
    pre_call_hook(&guardrail, &mut data, "embedding", CALLER)
        .await
        .expect("skipped");
    moderation_hook(&guardrail, &mut data, "aembedding", CALLER)
        .await
        .expect("skipped");
    assert!(captured.lock().expect("captured").is_empty());
    let mut inspecting = build(&base);
    inspecting.inspect_embeddings = true;
    pre_call_hook(&inspecting, &mut data, "embedding", CALLER)
        .await
        .expect("inspected");
    assert_eq!(captured.lock().expect("captured").len(), 1);
    server.shutdown().await;
}

#[tokio::test]
async fn pre_call_hook_raises_a_400_on_a_block_action() {
    let answer = action("block_action", Some("blocked the request"));
    let (base, mut server, _captured) = serve(axum::http::StatusCode::OK, answer).await;
    let mut data = chat_data();
    let original = data.clone();
    let error = pre_call_hook(&build(&base), &mut data, "completion", CALLER)
        .await
        .expect_err("blocked");
    server.shutdown().await;
    assert_eq!(data, original);
    assert!(matches!(
        error,
        CatoError::Blocked { ref message } if message == "blocked the request"
    ));
    assert_eq!(error.status_code().as_u16(), 400);
}

#[tokio::test]
async fn moderation_hook_rewrites_the_request_on_an_anonymize_action() {
    let answer = anonymized(&[object(&[
        ("content", text("REDACTED")),
        ("role", text("user")),
    ])]);
    let (base, mut server, _captured) = serve(axum::http::StatusCode::OK, answer).await;
    let mut data = body(&[(
        "messages",
        list(&[object(&[
            ("content", text("secret")),
            ("role", text("user")),
        ])]),
    )]);
    moderation_hook(&build(&base), &mut data, "completion", CALLER)
        .await
        .expect("rewritten");
    server.shutdown().await;
    assert_eq!(
        field_text(&array_of(&data, "messages"), "content"),
        ["REDACTED"]
    );
}

#[tokio::test]
async fn post_call_success_hook_redacts_the_assistant_text() {
    let answer = anonymized(&[
        object(&[("content", text("hello")), ("role", text("user"))]),
        object(&[("content", text("REDACTED")), ("role", text("assistant"))]),
    ]);
    let (base, mut server, captured) = serve(axum::http::StatusCode::OK, answer).await;
    let mut response = chat_response(&[("content", text("the secret answer"))]);
    post_call_success_hook(
        &build(&base),
        &chat_data(),
        CALLER,
        ResponseKind::Chat,
        &mut response,
    )
    .await
    .expect("inspected");
    let recorded = captured.lock().expect("captured").clone();
    server.shutdown().await;
    assert_eq!(
        first_message(&response).get("content"),
        Some(&text("REDACTED"))
    );
    let (headers, request_body) = recorded.first().expect("one request");
    assert_header(headers, "x-cato-litellm-hook", "output");
    assert_header(headers, "x-cato-call-id", "call-1");
    let posted: PostedBody = serde_json::from_str(request_body).expect("posted body");
    assert_eq!(
        field_text(&posted.messages, "content"),
        ["hello", "the secret answer"]
    );
    assert_eq!(field_text(&posted.messages, "role"), ["user", "assistant"]);
}

#[tokio::test]
async fn post_call_success_hook_passes_a_clean_answer_through() {
    let mut response = chat_response(&[("content", text("the answer"))]);
    let (result, posted) = output_hook(no_action(), &mut response, ResponseKind::Chat).await;
    result.expect("inspected");
    assert_eq!(posted.len(), 1);
    assert_eq!(
        first_message(&response).get("content"),
        Some(&text("the answer"))
    );
}

#[tokio::test]
async fn post_call_success_hook_raises_a_400_on_a_block_action() {
    let mut response = chat_response(&[("content", text("the answer"))]);
    let answer = action("block_action", Some("blocked the answer"));
    let (result, _posted) = output_hook(answer, &mut response, ResponseKind::Chat).await;
    let error = result.expect_err("blocked");
    assert_eq!(
        first_message(&response).get("content"),
        Some(&text("the answer"))
    );
    assert!(matches!(
        error,
        CatoError::Blocked { ref message } if message == "blocked the answer"
    ));
    assert_eq!(error.status_code().as_u16(), 400);
}

#[tokio::test]
async fn post_call_success_hook_leaves_an_answer_without_a_usable_redaction_alone() {
    let mut monitor = action("monitor_action", None);
    monitor.redacted_chat = Some(Redacted {
        all_redacted_messages: vec![object(&[("content", text("REDACTED"))])],
    });
    let answers = [
        anonymized(&[object(&[("content", text("REDACTED"))])]),
        anonymized(&[object(&[("role", text("assistant"))])]),
        monitor,
    ];
    for answer in answers {
        let mut response = chat_response(&[("content", text("the answer"))]);
        let (result, _posted) = output_hook(answer, &mut response, ResponseKind::Chat).await;
        result.expect("inspected");
        assert_eq!(
            first_message(&response).get("content"),
            Some(&text("the answer"))
        );
    }
}

#[tokio::test]
async fn post_call_success_hook_rejects_a_redaction_that_is_not_text() {
    let answer = anonymized(&[
        object(&[("content", text("hello"))]),
        object(&[("content", list(&[text("part")]))]),
    ]);
    let mut response = chat_response(&[("content", text("the answer"))]);
    let (result, _posted) = output_hook(answer, &mut response, ResponseKind::Chat).await;
    let error = result.expect_err("not text");
    assert!(matches!(error, CatoError::OutputRedactionNotText));
    assert_eq!(error.status_code().as_u16(), 502);
}

#[tokio::test]
async fn post_call_success_hook_inspects_an_empty_content_and_skips_empty_arguments() {
    let answer = anonymized(&[
        object(&[("content", text("hello"))]),
        object(&[("content", text("REDACTED"))]),
    ]);
    let mut response = chat_response(&[
        ("content", text("")),
        (
            "tool_calls",
            list(&[
                object(&[("function", object(&[("arguments", text(""))]))]),
                object(&[("function", object(&[("arguments", text("call-one"))]))]),
            ]),
        ),
    ]);
    let (result, posted) = output_hook(answer, &mut response, ResponseKind::Chat).await;
    result.expect("inspected");
    assert_eq!(posted.len(), 2);
    assert_eq!(field_text(&posted[0].messages, "content"), ["hello", ""]);
    assert_eq!(
        field_text(&posted[1].messages, "content"),
        ["hello", "call-one"]
    );
    let message = first_message(&response);
    assert_eq!(message.get("content"), Some(&text("REDACTED")));
    assert_eq!(argument(message, 0), Some(""));
    assert_eq!(argument(message, 1), Some("REDACTED"));
}

#[tokio::test]
async fn post_call_success_hook_skips_a_null_content_without_tool_calls() {
    let mut response = chat_response(&[
        ("content", GuardrailValue::Null),
        ("role", text("assistant")),
    ]);
    let (result, posted) = output_hook(no_action(), &mut response, ResponseKind::Chat).await;
    result.expect("inspected");
    assert!(posted.is_empty());
    assert_eq!(
        first_message(&response).get("content"),
        Some(&GuardrailValue::Null)
    );
}

#[tokio::test]
async fn post_call_success_hook_redacts_tool_call_arguments_without_a_content_key() {
    let answer = anonymized(&[
        object(&[("content", text("hello"))]),
        object(&[("content", text("REDACTED"))]),
    ]);
    let mut response = chat_response(&[(
        "tool_calls",
        list(&[object(&[(
            "function",
            object(&[("arguments", text("call-one"))]),
        )])]),
    )]);
    let (result, posted) = output_hook(answer, &mut response, ResponseKind::Chat).await;
    result.expect("inspected");
    assert_eq!(posted.len(), 1);
    let message = first_message(&response);
    assert!(!message.contains_key("content"));
    assert_eq!(argument(message, 0), Some("REDACTED"));
}

#[tokio::test]
async fn post_call_success_hook_redacts_responses_function_call_arguments() {
    let answer = anonymized(&[
        object(&[("content", text("hello"))]),
        object(&[("content", text("REDACTED"))]),
    ]);
    let mut response = object(&[(
        "output",
        list(&[
            object(&[
                ("type", text("function_call")),
                ("arguments", text("calc")),
                ("call_id", text("call-1")),
            ]),
            object(&[("type", text("function_call")), ("arguments", text(""))]),
            object(&[("type", text("reasoning"))]),
        ]),
    )]);
    let (result, posted) = output_hook(answer, &mut response, ResponseKind::Responses).await;
    result.expect("inspected");
    assert_eq!(
        field_text(&posted[0].messages, "content"),
        ["hello", "calc"]
    );
    let output = output_items(&response);
    let GuardrailValue::Object(function_call) = &output[0] else {
        panic!("the function call is not an object");
    };
    assert_eq!(function_call.get("arguments"), Some(&text("REDACTED")));
    assert_eq!(function_call.get("call_id"), Some(&text("call-1")));
    let GuardrailValue::Object(empty_call) = &output[1] else {
        panic!("the empty call is not an object");
    };
    assert_eq!(empty_call.get("arguments"), Some(&text("")));
    let GuardrailValue::Object(reasoning) = &output[2] else {
        panic!("the reasoning item is not an object");
    };
    assert!(!reasoning.contains_key("content"));
}

#[tokio::test]
async fn post_call_success_hook_redacts_responses_output_text() {
    let answer = anonymized(&[
        object(&[("content", text("hello"))]),
        object(&[("content", text("REDACTED"))]),
    ]);
    let content = list(&[
        object(&[("type", text("output_text")), ("text", text("hi"))]),
        object(&[("type", text("reasoning")), ("text", text("hidden"))]),
        object(&[("type", text("output_text")), ("text", text(""))]),
    ]);
    let mut response = object(&[(
        "output",
        list(&[object(&[("type", text("message")), ("content", content)])]),
    )]);
    let (result, posted) = output_hook(answer, &mut response, ResponseKind::Responses).await;
    result.expect("inspected");
    assert_eq!(field_text(&posted[0].messages, "content"), ["hello", "hi"]);
    let output = output_items(&response);
    let GuardrailValue::Object(message) = &output[0] else {
        panic!("the message is not an object");
    };
    assert_eq!(
        message.get("content"),
        Some(&list(&[
            object(&[("type", text("output_text")), ("text", text("REDACTED"))]),
            object(&[("type", text("reasoning")), ("text", text("hidden"))]),
            object(&[("type", text("output_text")), ("text", text(""))]),
        ]))
    );
}

#[test]
fn write_fragment_refuses_a_path_it_cannot_reach() {
    let mut response = chat_response(&[("content", text("hi"))]);
    let empty: &[Step] = &[];
    let last_is_an_index: &[Step] = &[Step::Key("choices"), Step::Index(0)];
    let index_into_an_object: &[Step] = &[
        Step::Key("choices"),
        Step::Index(0),
        Step::Index(0),
        Step::Key("content"),
    ];
    let text_parent: &[Step] = &[
        Step::Key("choices"),
        Step::Index(0),
        Step::Key("message"),
        Step::Key("content"),
        Step::Key("nested"),
    ];
    for path in [empty, last_is_an_index, index_into_an_object, text_parent] {
        assert!(matches!(
            write_fragment(&mut response, path, "x".to_owned()),
            Err(CatoError::OutputRedactionMismatch)
        ));
    }
    assert_eq!(first_message(&response).get("content"), Some(&text("hi")));
}
