use betterlite_core::api::assistant_read_delete::error::AssistantReadDeleteError;
use betterlite_core::api::assistant_read_delete::request::AssistantOperation;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::AssistantSdkError;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::ExposeSecret as _;

use super::{request_create, request_read_delete, request_settings, request_thread};
use crate::assistant_calls::error::{family, payload};

const READ_DELETE: &str = r#"{"operation":"list","provider":"openai","assistant_id":null,
    "aget_assistants":true,"async_delete_assistants":null,"after":"a","before":null,
    "limit":20,"order":"desc","params":{"api_key":"sk-param","timeout":1.5}}"#;

#[test]
fn request_settings_keeps_every_module_global() {
    let text = r#"{"api_key":"sk-a","api_base":"http://base","api_version":"v1",
        "organization":"org","openai_key":"sk-b","azure_key":"az"}"#;
    let settings = request_settings(text).unwrap();
    assert_eq!(settings.api_key.as_deref(), Some("sk-a"));
    assert_eq!(settings.api_base.as_deref(), Some("http://base"));
    assert_eq!(settings.api_version.as_deref(), Some("v1"));
    assert_eq!(settings.organization.as_deref(), Some("org"));
    assert_eq!(settings.openai_key.as_deref(), Some("sk-b"));
    assert_eq!(settings.azure_key.as_deref(), Some("az"));
}

#[test]
fn request_read_delete_keeps_the_flag_params_and_pagination() {
    let input = request_read_delete(READ_DELETE).unwrap();
    assert_eq!(input.operation, AssistantOperation::List);
    assert_eq!(input.aget_assistants, Some(Json::Bool(true)));
    assert_eq!(input.async_delete_assistants, None);
    assert_eq!(input.limit, Some(20));
    assert_eq!(input.order.as_deref(), Some("desc"));
    assert_eq!(
        input.params.api_key.as_ref().map(|key| key.expose_secret()),
        Some("sk-param")
    );
}

#[test]
fn request_read_delete_refuses_a_key_the_facade_does_not_read() {
    let text = r#"{"operation":"delete","provider":"openai","bucket":1}"#;
    let error = request_read_delete(text).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn request_create_keeps_a_zero_temperature_and_the_tools() {
    let text = r#"{"provider":"openai","model":"gpt-4o-mini","temperature":0,"top_p":0,
        "tools":[{"type":"code_interpreter"}],"metadata":{"team":"a"}}"#;
    let input = request_create(text).unwrap();
    assert_eq!(input.temperature.unwrap().as_f64(), Some(0.0_f64));
    assert_eq!(input.top_p.unwrap().as_f64(), Some(0.0_f64));
    assert_eq!(input.tools.unwrap().len(), 1);
    assert_eq!(input.metadata.unwrap().len(), 1);
}

#[test]
fn request_thread_keeps_messages_metadata_and_kwargs() {
    let text = r#"{"custom_llm_provider":"openai","thread_id":null,
        "messages":[{"role":"user","content":"hi"}],"metadata":{"a":"b"},
        "tool_resources":null,"kwargs":{"request_timeout":5,"extra_body":null}}"#;
    let request = request_thread(text).unwrap();
    assert_eq!(request.custom_llm_provider, "openai");
    assert_eq!(request.thread_id, None);
    assert_eq!(request.messages.unwrap().len(), 1);
    assert_eq!(request.metadata.unwrap().len(), 1);
    assert_eq!(request.tool_resources, None);
    assert_eq!(request.kwargs.len(), 2);
}

#[test]
fn error_payload_for_an_unsupported_provider_names_its_class_and_model() {
    let error: AssistantReadDeleteError<AssistantSdkError> =
        AssistantReadDeleteError::Unsupported {
            provider: String::from("bedrock"),
            function: "get_assistants",
        };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let text = serde_json::to_string(&family::read_delete(error, "bedrock", &redactor)).unwrap();
    assert_eq!(
        text,
        r#"{"type":"BadRequestError","message":"LiteLLM doesn't support bedrock for 'get_assistants'. Only 'openai' is supported.","status_code":400,"llm_provider":"bedrock","model":"n/a"}"#
    );
}

#[test]
fn error_payload_for_a_plain_exception_carries_no_status() {
    let message =
        String::from("Invalid value passed in for aget_assistants. Only bool or None allowed");
    let text = serde_json::to_string(&payload("Exception", message)).unwrap();
    assert_eq!(
        text,
        r#"{"type":"Exception","message":"Invalid value passed in for aget_assistants. Only bool or None allowed"}"#
    );
}
