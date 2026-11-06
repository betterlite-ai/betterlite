#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_http::proxy::base_builder;
use reqwest::StatusCode;
use secrecy::SecretString;

pub(crate) mod support;

use crate::pangea::config::{PangeaGuardrailConfigModel, PangeaOptionalParams};
use crate::pangea::error::PangeaGuardrailError;
use crate::pangea::hooks::{
    CallbackRegistry, GuardrailResponse, PangeaGuardrail, RequestData, SUPPORTED_EVENT_HOOKS,
    initialize_guardrail,
};
use crate::pangea::tests::support::{
    GUARDRAIL_NAME, Recorder, config, data_with_messages, guardrail, payload, serve, serve_hanging,
    user,
};

const ALLOWED: &str = r#"{"result":{"blocked":false,"transformed":false}}"#;

fn tool_definition() -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([
        (
            "type".to_owned(),
            GuardrailValue::Text("function".to_owned()),
        ),
        (
            "function".to_owned(),
            GuardrailValue::Object(BTreeMap::from([(
                "name".to_owned(),
                GuardrailValue::Text("lookup".to_owned()),
            )])),
        ),
    ]))
}

fn message_choice(content: &str) -> GuardrailValue {
    GuardrailValue::Object(BTreeMap::from([(
        "message".to_owned(),
        GuardrailValue::Object(BTreeMap::from([
            (
                "role".to_owned(),
                GuardrailValue::Text("assistant".to_owned()),
            ),
            (
                "content".to_owned(),
                GuardrailValue::Text(content.to_owned()),
            ),
        ])),
    )]))
}

#[test]
fn supported_event_hooks_are_pre_call_and_post_call() {
    assert_eq!(
        serde_json::to_string(&SUPPORTED_EVENT_HOOKS).unwrap(),
        r#"["pre_call","post_call"]"#
    );
}

#[tokio::test]
async fn pre_call_sends_chat_messages_tools_and_the_input_event_type() {
    let server = serve(StatusCode::OK, ALLOWED.to_owned()).await;
    let instance = guardrail(&server.base);
    let messages = vec![user("hello")];
    let mut data = RequestData {
        messages: Some(messages.clone()),
        prompt: None,
        tools: Some(tool_definition()),
    };
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
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
    assert!(!sent.debug);
    assert_eq!(sent.event_type, "input");
    assert_eq!(sent.input.messages, Some(messages));
    assert_eq!(sent.input.tools, Some(tool_definition()));
    assert_eq!(sent.input.choices, None);
    assert_eq!(sent.recipe, None);
    assert_eq!(recorder.names, [GUARDRAIL_NAME]);
    assert_eq!(data.messages, Some(vec![user("hello")]));
}

#[tokio::test]
async fn pre_call_sends_the_configured_input_recipe() {
    let server = serve(StatusCode::OK, ALLOWED.to_owned()).await;
    let mut instance = guardrail(&server.base);
    instance.input_recipe = Some("guard_llm_request".to_owned());
    let mut data = data_with_messages(vec![user("hello")]);
    let mut recorder = Recorder::default();
    instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
        .await
        .unwrap();
    assert_eq!(
        payload(&server.captured()).recipe.as_deref(),
        Some("guard_llm_request")
    );
}

#[tokio::test]
async fn pre_call_replaces_the_messages_when_the_answer_is_transformed() {
    let answer = r#"{"result":{"transformed":true,"output":{"messages":[{"role":"user","content":"masked"}]}}}"#;
    let server = serve(StatusCode::OK, answer.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("secret")]);
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
        .await
        .unwrap();
    assert!(transformed);
    assert_eq!(data.messages, Some(vec![user("masked")]));
}

#[tokio::test]
async fn pre_call_blocked_raises_a_bad_request_with_the_pangea_detail() {
    let blocked = r#"{"result":{"blocked":true,"transformed":false}}"#;
    let server = serve(StatusCode::OK, blocked.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("attack")]);
    let mut recorder = Recorder::default();
    let error = instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
        .await
        .expect_err("a blocked request fails");
    assert!(matches!(error, PangeaGuardrailError::Blocked));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    let detail = error.detail(GUARDRAIL_NAME);
    assert_eq!(detail.error, "Violated Pangea guardrail policy");
    assert_eq!(detail.guardrail_name, GUARDRAIL_NAME);
    assert_eq!(detail.exceptions, None);
    assert!(recorder.names.is_empty());
}

#[tokio::test]
async fn backend_http_error_is_a_server_error_with_the_backend_detail() {
    let server = serve(StatusCode::INTERNAL_SERVER_ERROR, "boom".to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("hello")]);
    let mut recorder = Recorder::default();
    let error = instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
        .await
        .expect_err("a backend failure fails the hook");
    assert!(matches!(
        error,
        PangeaGuardrailError::HttpStatus { status } if status == StatusCode::INTERNAL_SERVER_ERROR
    ));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    let message = error.to_string();
    let detail = error.detail(GUARDRAIL_NAME);
    assert_eq!(detail.error, "Error in Pangea Guardrail");
    assert_eq!(detail.exceptions, Some(message));
    assert!(recorder.names.is_empty());
}

#[tokio::test]
async fn backend_answer_that_is_not_json_is_a_server_error() {
    let server = serve(StatusCode::OK, "not json".to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("hello")]);
    let mut recorder = Recorder::default();
    let error = instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
        .await
        .expect_err("a malformed answer fails the hook");
    assert!(matches!(error, PangeaGuardrailError::ResponseJson { .. }));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn backend_answer_without_a_result_object_passes_the_request() {
    let server = serve(StatusCode::OK, "{}".to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("hello")]);
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
        .await
        .unwrap();
    assert!(!transformed);
    assert_eq!(data.messages, Some(vec![user("hello")]));
    assert_eq!(recorder.names, [GUARDRAIL_NAME]);
}

#[tokio::test]
async fn backend_answer_larger_than_the_cap_is_a_server_error() {
    let server = serve(StatusCode::OK, "x".repeat(1024 * 1024 + 1)).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("hello")]);
    let mut recorder = Recorder::default();
    let error = instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
        .await
        .expect_err("an oversized answer is refused");
    assert!(matches!(
        error,
        PangeaGuardrailError::ResponseTooLarge { .. }
    ));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn backend_timeout_is_a_server_error() {
    let server = serve_hanging().await;
    let mut instance = guardrail(&server.base);
    instance.client = base_builder()
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let mut data = data_with_messages(vec![user("hello")]);
    let mut recorder = Recorder::default();
    let error = instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
        .await
        .expect_err("the backend never answers");
    let PangeaGuardrailError::Request { source } = error else {
        panic!("a timed-out request is a transport failure");
    };
    assert!(source.is_timeout());
}

#[tokio::test]
async fn pre_call_transformed_without_messages_is_a_server_error() {
    let answer = r#"{"result":{"transformed":true}}"#;
    let server = serve(StatusCode::OK, answer.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("hello")]);
    let mut recorder = Recorder::default();
    let error = instance
        .async_pre_call_hook(&mut data, "completion", &mut recorder)
        .await
        .expect_err("a transformed answer names messages");
    assert!(matches!(
        error,
        PangeaGuardrailError::MissingData { field: "messages" }
    ));
    assert_eq!(recorder.names, [GUARDRAIL_NAME]);
}

#[tokio::test]
async fn post_call_sends_the_request_messages_and_the_response_choices() {
    let server = serve(StatusCode::OK, ALLOWED.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = RequestData {
        messages: Some(vec![user("hello")]),
        prompt: None,
        tools: Some(tool_definition()),
    };
    let mut response = GuardrailResponse::Model {
        choices: vec![message_choice("hi")],
    };
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_post_call_success_hook(&mut data, &mut response, &mut recorder)
        .await
        .unwrap();
    assert!(!transformed);
    let sent = payload(&server.captured());
    assert_eq!(sent.event_type, "output");
    assert_eq!(sent.input.messages, Some(vec![user("hello")]));
    assert_eq!(sent.input.tools, Some(tool_definition()));
    assert_eq!(sent.input.choices, Some(vec![message_choice("hi")]));
    assert_eq!(recorder.names, [GUARDRAIL_NAME]);
}

#[tokio::test]
async fn post_call_without_request_messages_skips_the_call() {
    let instance = guardrail("http://127.0.0.1:1");
    let mut data = RequestData {
        messages: None,
        prompt: None,
        tools: None,
    };
    let mut response = GuardrailResponse::Model {
        choices: vec![message_choice("hi")],
    };
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_post_call_success_hook(&mut data, &mut response, &mut recorder)
        .await
        .unwrap();
    assert!(!transformed);
    assert!(recorder.names.is_empty());
}

#[tokio::test]
async fn post_call_replaces_the_response_choices_when_transformed() {
    let answer = r#"{"result":{"transformed":true,"output":{"choices":[{"message":{"role":"assistant","content":"masked"}}]}}}"#;
    let server = serve(StatusCode::OK, answer.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("hello")]);
    let mut response = GuardrailResponse::Model {
        choices: vec![message_choice("secret")],
    };
    let mut recorder = Recorder::default();
    let transformed = instance
        .async_post_call_success_hook(&mut data, &mut response, &mut recorder)
        .await
        .unwrap();
    assert!(transformed);
    assert_eq!(
        response,
        GuardrailResponse::Model {
            choices: vec![message_choice("masked")]
        }
    );
}

#[tokio::test]
async fn post_call_blocked_raises_a_bad_request() {
    let blocked = r#"{"result":{"blocked":true}}"#;
    let server = serve(StatusCode::OK, blocked.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("hello")]);
    let mut response = GuardrailResponse::Model {
        choices: vec![message_choice("leak")],
    };
    let mut recorder = Recorder::default();
    let error = instance
        .async_post_call_success_hook(&mut data, &mut response, &mut recorder)
        .await
        .expect_err("a blocked response fails");
    assert!(matches!(error, PangeaGuardrailError::Blocked));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert!(recorder.names.is_empty());
}

#[tokio::test]
async fn post_call_transformed_without_choices_is_a_server_error() {
    let answer = r#"{"result":{"transformed":true,"output":{"messages":[]}}}"#;
    let server = serve(StatusCode::OK, answer.to_owned()).await;
    let instance = guardrail(&server.base);
    let mut data = data_with_messages(vec![user("hello")]);
    let mut response = GuardrailResponse::Model {
        choices: Vec::new(),
    };
    let mut recorder = Recorder::default();
    let error = instance
        .async_post_call_success_hook(&mut data, &mut response, &mut recorder)
        .await
        .expect_err("a transformed answer names choices");
    assert!(matches!(
        error,
        PangeaGuardrailError::MissingData { field: "choices" }
    ));
}

#[derive(Default)]
struct Registry {
    names: Vec<String>,
}

impl CallbackRegistry for Registry {
    fn add_litellm_callback(&mut self, guardrail: Arc<PangeaGuardrail>) {
        self.names.push(guardrail.guardrail_name.clone());
    }
}

#[test]
fn initialize_guardrail_registers_the_callback_it_returns() {
    let lookup = BTreeMap::from([("PANGEA_API_KEY".to_owned(), "env-key".to_owned())]);
    let mut registry = Registry::default();
    let guardrail = initialize_guardrail(
        Some(GUARDRAIL_NAME.to_owned()),
        config(),
        &lookup,
        reqwest::Client::default(),
        &mut registry,
    )
    .unwrap();
    assert_eq!(guardrail.guardrail_name, GUARDRAIL_NAME);
    assert_eq!(registry.names, [GUARDRAIL_NAME]);
}

#[test]
fn initialize_guardrail_requires_a_guardrail_name() {
    let lookup = BTreeMap::from([("PANGEA_API_KEY".to_owned(), "env-key".to_owned())]);
    let mut registry = Registry::default();
    let error = initialize_guardrail(
        None,
        config(),
        &lookup,
        reqwest::Client::default(),
        &mut registry,
    )
    .expect_err("a nameless guardrail does not initialize");
    assert!(matches!(error, PangeaGuardrailError::MissingGuardrailName));
    assert_eq!(error.to_string(), "Pangea guardrail name is required");
    assert!(registry.names.is_empty());
}

#[test]
fn guardrail_fields_are_built_from_the_parameters() {
    let config = PangeaGuardrailConfigModel {
        api_key: Some(SecretString::from("config-key".to_owned())),
        api_base: Some("https://pangea.test".to_owned()),
        optional_params: Some(PangeaOptionalParams {
            pangea_input_recipe: Some("in".to_owned()),
            pangea_output_recipe: Some("out".to_owned()),
        }),
    };
    let instance = PangeaGuardrail::new(
        Some(GUARDRAIL_NAME.to_owned()),
        config,
        &BTreeMap::new(),
        reqwest::Client::default(),
    )
    .unwrap();
    assert_eq!(instance.input_recipe.as_deref(), Some("in"));
    assert_eq!(instance.output_recipe.as_deref(), Some("out"));
    assert_eq!(instance.api_base, "https://pangea.test");
}
