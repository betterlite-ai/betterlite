use std::collections::BTreeSet;

use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use serde_json::Number;

use super::support::{
    self, BASE, KEY, REPLY, chat_http, field, first_content, loopback, proxied_client, serve_once,
};
use crate::foundry_dispatch::{AzureAi, AzureAiRequest};
use crate::operation::{ChatCompletion, ChatOperation, OperationRequest, OperationResult};
use crate::provider::Adapter as _;

const fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

// the optional parameters reach the body, and api_version becomes the URL query alone
fn chat_operation() -> OperationRequest {
    OperationRequest::Chat(Box::new(ChatOperation {
        model: String::from("mistral-large"),
        request: support::request(
            r#"{"model":"mistral-large","messages":[{"role":"user","content":"hi"}],"max_tokens":7,"api_version":"2024-02-01"}"#,
        ),
        control: ControlOptions::default(),
    }))
}

// Runs prepare, execute and decode against a loopback server and returns the reply and the raw request
fn loopback_exchange() -> (OperationResult, String) {
    support::block_on(async {
        let (listener, api_base) = loopback().await;
        let deployment = support::deployment(&api_base, Some(KEY));
        let prepared = AzureAi
            .prepare(&chat_operation(), &deployment, &catalog())
            .expect("the adapter prepares the chat call");
        let expected = chat_http(&prepared).body.clone();
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{REPLY}",
            REPLY.len()
        );
        let client = proxied_client();
        let (upstream, sent) = tokio::join!(
            AzureAi.execute(&client, &prepared, &deployment),
            serve_once(listener, &expected, &reply)
        );
        let decoded = AzureAi
            .decode(&prepared, upstream.expect("the loopback dispatch runs"))
            .expect("the adapter decodes the reply");
        (decoded, sent)
    })
}

fn assert_loopback_request(sent: &str) {
    assert!(sent.starts_with("POST /models/chat/completions?api-version=2024-02-01 HTTP/1.1"));
    let lower = sent.to_lowercase();
    assert!(lower.contains("authorization: bearer sk-foundry-fixture"));
    assert!(lower.contains("content-type: application/json"));
    let raw = sent.split("\r\n\r\n").nth(1).expect("the request body");
    let body: JsonValue = serde_json::from_str(raw).expect("the request body");
    assert_eq!(
        field(&body, "model"),
        &JsonValue::Text(String::from("mistral-large"))
    );
    assert_eq!(
        field(&body, "max_tokens"),
        &JsonValue::Number(Number::from(7_u8))
    );
    assert_eq!(first_content(&body), &JsonValue::Text(String::from("hi")));
}

pub(super) fn http_case() {
    let (decoded, sent) = loopback_exchange();
    let OperationResult::Chat(ChatCompletion::OpenAi(completion)) = decoded else {
        panic!("the loopback dispatch returns a chat reply");
    };
    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("hello")
    );
    assert_loopback_request(&sent);
    adapter_refuses_other_kinds();
}

// the adapter refuses the operation kinds it does not run
fn adapter_refuses_other_kinds() {
    let deployment = support::deployment(BASE, Some(KEY));
    let error = AzureAi
        .prepare(
            &OperationRequest::AzureAi(AzureAiRequest),
            &deployment,
            &catalog(),
        )
        .expect_err("the other kind is refused");
    assert_eq!(
        error.to_string(),
        "betterlite does not run the provider operation through the azure_ai provider"
    );
}
