#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::resource_sdk_settings;
use betterlite_core::api::assistant_messages::error::{
    MessageError, SetupError, failure as message_failure,
};
use betterlite_core::api::assistant_run_sdk::call::RunFlag;
use betterlite_core::api::assistant_run_sdk::error::AssistantRunError;
use betterlite_error::envelope::WireField;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::{
    MessageCall, ModuleSettings, RunCall, inner_message, message_error, read, run_error, wire_body,
};

const APPEND: &str = r#"{"custom_llm_provider":"openai","thread_id":"thread_1",
    "data":{"role":"user","content":"hello","attachments":[{"file_id":"file_1"}],
    "metadata":{"team":"a"}},"params":{"api_key":"sk-param"},"drop_params":true}"#;

#[test]
fn request_settings_keeps_every_module_global() {
    let text = r#"{"api_key":"sk-a","api_base":"http://base","api_version":"v1",
        "organization":"org","openai_key":"sk-b","azure_key":"az"}"#;
    let settings: ModuleSettings = read("assistant session settings", text).unwrap();
    let api_key = settings
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    let openai_key = settings
        .openai_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    let azure_key = settings
        .azure_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    assert_eq!(api_key, Some("sk-a"));
    assert_eq!(settings.api_base.as_deref(), Some("http://base"));
    assert_eq!(settings.api_version.as_deref(), Some("v1"));
    assert_eq!(settings.organization.as_deref(), Some("org"));
    assert_eq!(openai_key, Some("sk-b"));
    assert_eq!(azure_key, Some("az"));
}

#[test]
fn request_settings_replace_the_environment_values() {
    let mut settings = resource_sdk_settings::Raw::default()
        .convert(&BTreeMap::<String, String>::new())
        .unwrap();
    let text = r#"{"api_key":"sk-a","api_base":"http://base","api_version":"v1",
        "organization":"org","openai_key":"sk-b","azure_key":"az"}"#;
    let module: ModuleSettings = read("assistant session settings", text).unwrap();
    module.apply(&mut settings);
    assert_eq!(settings.api_base.as_deref(), Some("http://base"));
    assert_eq!(settings.api_version.as_deref(), Some("v1"));
    assert_eq!(settings.organization.as_deref(), Some("org"));
    assert!(settings.api_key.is_some());
    assert!(settings.openai_key.is_some());
    assert!(settings.azure_key.is_some());
}

#[test]
fn request_append_call_reads_the_named_arguments() {
    let call: MessageCall = read("assistant add message request", APPEND).unwrap();
    let data = call.data.as_ref().unwrap();
    assert_eq!(call.custom_llm_provider, "openai");
    assert_eq!(call.thread_id, "thread_1");
    assert_eq!(data.role, "user");
    assert_eq!(data.content.as_str(), Some("hello"));
    assert_eq!(
        serde_json::to_string(&data.attachments).unwrap(),
        r#"[{"file_id":"file_1"}]"#
    );
    assert_eq!(
        serde_json::to_string(&data.metadata).unwrap(),
        r#"{"team":"a"}"#
    );
    assert!(call.drop_params);
    let api_key = call
        .params
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    assert_eq!(api_key, Some("sk-param"));
}

#[test]
fn request_append_call_defaults_the_optional_arguments() {
    let text = r#"{"custom_llm_provider":"azure","thread_id":"thread_1",
        "data":{"role":"user","content":""}}"#;
    let call: MessageCall = read("assistant add message request", text).unwrap();
    let data = call.data.as_ref().unwrap();
    assert_eq!(data.content.as_str(), Some(""));
    assert_eq!(data.attachments, None);
    assert_eq!(data.metadata, None);
    assert!(!call.drop_params);
    assert!(call.params.api_key.is_none());
}

#[test]
fn request_append_call_refuses_a_key_the_facade_does_not_read() {
    let text = r#"{"custom_llm_provider":"openai","thread_id":"t",
        "data":{"role":"user","content":"hi"},"assistant_id":"asst"}"#;
    let error = read::<MessageCall>("assistant add message request", text).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn request_list_call_requires_the_thread() {
    let text = r#"{"custom_llm_provider":"openai"}"#;
    let error = read::<MessageCall>("assistant get messages request", text).unwrap_err();
    assert!(error.to_string().contains("missing field `thread_id`"));
}

#[test]
fn request_list_call_reads_the_connection_parameters() {
    let text = r#"{"custom_llm_provider":"openai","thread_id":"thread_1",
        "params":{"api_base":"http://base","timeout":2.5}}"#;
    let call: MessageCall = read("assistant get messages request", text).unwrap();
    assert_eq!(call.thread_id, "thread_1");
    assert_eq!(call.params.api_base.as_deref(), Some("http://base"));
    assert!(call.data.is_none());
}

#[test]
fn request_run_call_keeps_an_absent_stream_apart_from_a_null_one() {
    let base = r#"{"custom_llm_provider":"openai","thread_id":"t","assistant_id":"asst"}"#;
    let absent: RunCall = read("assistant run request", base).unwrap();
    assert_eq!(absent.stream, None);
    let nulled: RunCall = read(
        "assistant run request",
        &base.replace('}', r#","stream":null}"#),
    )
    .unwrap();
    assert_eq!(nulled.stream, Some(None));
    let flagged: RunCall = read(
        "assistant run request",
        &base.replace('}', r#","stream":true}"#),
    )
    .unwrap();
    assert_eq!(flagged.stream, Some(Some(RunFlag::Bool(true))));
}

#[test]
fn request_run_call_carries_the_named_arguments_into_the_call() {
    let text = r#"{"custom_llm_provider":"openai","thread_id":"thread_1",
        "assistant_id":"asst_1","additional_instructions":"extra","instructions":null,
        "metadata":{"a":"b"},"model":"gpt-4o-mini","stream":false,
        "tools":[{"type":"code_interpreter"}],"params":{"api_version":"v2"}}"#;
    let call = read::<RunCall>("assistant run request", text)
        .unwrap()
        .into_call();
    assert_eq!(call.provider, "openai");
    assert_eq!(call.thread_id, "thread_1");
    assert_eq!(call.assistant_id, "asst_1");
    assert_eq!(call.additional_instructions.as_deref(), Some("extra"));
    assert_eq!(call.instructions, None);
    assert_eq!(
        serde_json::to_string(&call.metadata).unwrap(),
        r#"{"a":"b"}"#
    );
    assert_eq!(call.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(call.stream, Some(Some(RunFlag::Bool(false))));
    assert_eq!(
        serde_json::to_string(&call.tools).unwrap(),
        r#"[{"type":"code_interpreter"}]"#
    );
    assert_eq!(call.params.api_version.as_deref(), Some("v2"));
    assert!(call.client.is_none());
    assert!(call.event_handler.is_none());
    assert!(call.arun_thread.is_none());
}

#[test]
fn error_message_error_names_the_model_an_unsupported_provider_answers() {
    let error = MessageError::Setup(SetupError::UnsupportedProvider {
        provider: String::from("bedrock"),
        operation: "create_thread",
    });
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let failure = message_failure(error, "bedrock", &redactor);
    let answer = message_error(&failure, "bedrock");
    assert_eq!(answer.class, "BadRequestError");
    assert_eq!(
        answer.message,
        "LiteLLM doesn't support bedrock for 'create_thread'. Only 'openai' is supported."
    );
    assert_eq!(answer.status_code, Some(400));
    assert_eq!(answer.llm_provider.as_deref(), Some("bedrock"));
    assert_eq!(answer.model.as_deref(), Some("n/a"));
    assert_eq!(
        serde_json::to_string(&answer.body).unwrap(),
        r#"{"type":null,"param":null}"#
    );
}

#[test]
fn error_message_error_answers_a_wrong_reply_as_a_plain_exception() {
    let error = MessageError::UnexpectedReply {
        operation: "get_messages",
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let failure = message_failure(error, "openai", &redactor);
    let answer = message_error(&failure, "openai");
    assert_eq!(answer.class, "Exception");
    assert_eq!(
        answer.message,
        "the provider answered the get_messages call with the wrong SDK object"
    );
    assert_eq!(answer.status_code, None);
    assert_eq!(answer.llm_provider.as_deref(), Some("openai"));
    assert_eq!(answer.model, None);
}

#[test]
fn error_run_error_reports_an_unsupported_provider_as_a_bad_request() {
    let error = AssistantRunError::Unsupported {
        provider: String::from("bedrock"),
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let answer = run_error(error, "openai", &redactor);
    assert_eq!(answer.class, "BadRequestError");
    assert_eq!(
        answer.message,
        "LiteLLM doesn't support bedrock for 'run_thread'. Only 'openai' is supported."
    );
    assert_eq!(answer.status_code, Some(400));
    assert_eq!(answer.llm_provider.as_deref(), Some("bedrock"));
    assert_eq!(answer.model.as_deref(), Some("n/a"));
}

#[test]
fn error_run_error_reports_a_duplicate_stream_as_a_plain_exception() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let answer = run_error(AssistantRunError::DuplicateStream, "openai", &redactor);
    assert_eq!(answer.class, "Exception");
    assert_eq!(
        answer.message,
        "litellm.assistants.main.run_thread() got multiple values for keyword argument 'stream'"
    );
    assert_eq!(answer.status_code, None);
    assert_eq!(answer.llm_provider, None);
}

#[test]
fn error_wire_body_keeps_a_null_field_and_drops_a_missing_one() {
    let null = WireField::Null;
    let text = WireField::Text(String::from("invalid_api_key"));
    let body = wire_body([
        ("type", &WireField::MISSING),
        ("param", &null),
        ("code", &text),
    ]);
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"param":null,"code":"invalid_api_key"}"#
    );
    let empty = wire_body([
        ("type", &WireField::MISSING),
        ("param", &WireField::MISSING),
        ("code", &WireField::MISSING),
    ]);
    assert!(empty.is_none());
}

#[test]
fn error_inner_message_strips_the_matching_prefix_once() {
    assert_eq!(
        inner_message(
            "BadRequestError",
            String::from("litellm.BadRequestError: bad input")
        ),
        "bad input"
    );
    assert_eq!(
        inner_message("BadRequestError", String::from("litellm.Timeout: slow")),
        "litellm.Timeout: slow"
    );
}

#[test]
fn error_negative_timeout_becomes_the_timeout_message() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let error = AssistantRunError::InvalidTimeout { seconds: -1.0_f64 };
    let answer = run_error(error, "openai", &redactor);
    assert_eq!(answer.class, "Timeout");
    assert_eq!(
        answer.message,
        "APITimeoutError - Request timed out. Error_str: Request timed out."
    );
    assert_eq!(answer.status_code, Some(408));
}

#[test]
fn error_timeout_text_keeps_its_text_and_no_status() {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let error = AssistantRunError::InvalidTimeoutText {
        text: String::from("soon"),
    };
    let answer = run_error(error, "openai", &redactor);
    assert_eq!(answer.class, "APIConnectionError");
    assert!(
        answer
            .message
            .contains("could not convert string to float: 'soon'")
    );
    assert_ne!(answer.status_code, Some(408));
}
