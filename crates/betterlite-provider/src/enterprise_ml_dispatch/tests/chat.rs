use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_openai::sagemaker_completion::request::CustomPromptEntry;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::data_cloud_dispatch::call::CompletionCall;
use crate::enterprise_ml_dispatch::chat::{Completion, SAP_MODEL_VERSION};
use crate::enterprise_ml_dispatch::error::EnterpriseError;
use crate::enterprise_ml_dispatch::sagemaker::SagemakerReply;

use super::support::{
    Chat, Recorder, Sagemaker, Scripted, at, dispatch, exposed, identifier, key, message, object,
};

fn text_call(chat: &Chat) -> CompletionCall {
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    dispatch(ChatConfigTag::IbmWatsonXAi, chat, &sagemaker, &recorder)
        .expect("the text arm runs")
        .expect("the handler answers");
    recorder.calls()[0].clone()
}

fn sap_headers(
    headers: Option<BTreeMap<String, String>>,
    globals: Option<BTreeMap<String, String>>,
) -> Option<BTreeMap<String, String>> {
    let mut chat = Chat::new("sap", "gpt-4o");
    chat.headers = headers;
    chat.globals_headers = globals;
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    dispatch(
        ChatConfigTag::GenAiHubOrchestration,
        &chat,
        &sagemaker,
        &recorder,
    )
    .expect("the sap arm runs")
    .expect("the handler answers");
    recorder.calls()[0].headers.clone()
}

#[test]
fn dispatch_leaves_the_other_providers_alone() {
    let chat = Chat::new("openai", "gpt-4o");
    let recorder = Recorder::answer("unused");
    let sagemaker = Sagemaker::new(Scripted::empty());
    let gpt = dispatch(ChatConfigTag::OpenAiGpt, &chat, &sagemaker, &recorder);
    assert!(gpt.is_none());
    let gpt5 = dispatch(ChatConfigTag::OpenAiGpt5, &chat, &sagemaker, &recorder);
    assert!(gpt5.is_none());
    assert!(recorder.calls().is_empty());
}

#[test]
fn sap_chat_merges_the_default_model_version() {
    let chat = Chat::new("sap", "gpt-4o");
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    let outcome = dispatch(
        ChatConfigTag::GenAiHubOrchestration,
        &chat,
        &sagemaker,
        &recorder,
    )
    .expect("the sap arm runs")
    .expect("the handler answers");
    assert!(matches!(outcome, Completion::Handler(reply) if reply == "reply"));
    let sent = recorder.calls();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].optional_params,
        object(&format!(r#"{{"model_version":"{SAP_MODEL_VERSION}"}}"#))
    );
    assert_eq!(sent[0].custom_llm_provider, "sap");
}

fn sap_model_version(params: Object) -> Object {
    let mut chat = Chat::new("sap", "gpt-4o");
    chat.optional_params = params;
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    dispatch(
        ChatConfigTag::GenAiHubOrchestration,
        &chat,
        &sagemaker,
        &recorder,
    )
    .expect("the sap arm runs")
    .expect("the handler answers");
    recorder.calls()[0].optional_params.clone()
}

#[test]
fn sap_chat_keeps_a_model_version_the_caller_set() {
    assert_eq!(
        sap_model_version(object(r#"{"model_version":"2405"}"#)),
        object(r#"{"model_version":"2405"}"#)
    );
}

#[test]
fn sap_chat_replaces_a_null_model_version() {
    assert_eq!(
        sap_model_version(object(r#"{"model_version":null}"#)),
        object(&format!(r#"{{"model_version":"{SAP_MODEL_VERSION}"}}"#))
    );
}

#[test]
fn sap_chat_falls_back_to_the_global_headers() {
    let global = BTreeMap::from([(String::from("x-global"), String::from("1"))]);
    let headers = sap_headers(Some(BTreeMap::new()), Some(global.clone()));
    assert_eq!(headers, Some(global));
}

#[test]
fn sap_chat_keeps_the_caller_headers() {
    let explicit = BTreeMap::from([(String::from("x-explicit"), String::from("2"))]);
    let global = BTreeMap::from([(String::from("x-global"), String::from("1"))]);
    let headers = sap_headers(Some(explicit.clone()), Some(global));
    assert_eq!(headers, Some(explicit));
}

#[test]
fn sap_chat_reports_the_handler_failure() {
    let chat = Chat::new("sap", "gpt-4o");
    let sagemaker = Sagemaker::new(Scripted::empty());
    let error = dispatch(
        ChatConfigTag::GenAiHubOrchestration,
        &chat,
        &sagemaker,
        &Recorder::failing(502),
    )
    .expect("the sap arm runs")
    .expect_err("the handler fails");
    assert!(matches!(error, EnterpriseError::Handler(_)));
    assert_eq!(error.status(), Some(502));
}

#[test]
fn watsonx_chat_passes_the_context_fields_under_the_literal_provider() {
    let mut chat = Chat::new("watsonx_chat", "ibm/granite");
    chat.api_base = Some(String::from("https://wx.example.com"));
    chat.api_key = Some(key("ctx-key"));
    chat.optional_params = object(r#"{"temperature":0.2}"#);
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    dispatch(ChatConfigTag::IbmWatsonXChat, &chat, &sagemaker, &recorder)
        .expect("the watsonx arm runs")
        .expect("the handler answers");
    let sent = recorder.calls();
    assert_eq!(sent[0].custom_llm_provider, "watsonx");
    assert_eq!(sent[0].api_base.as_deref(), Some("https://wx.example.com"));
    assert_eq!(
        exposed(sent[0].api_key.as_ref()).as_deref(),
        Some("ctx-key")
    );
    assert_eq!(sent[0].optional_params, object(r#"{"temperature":0.2}"#));
}

#[test]
fn watsonx_text_keeps_the_parameter_keys_the_context_overrides() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.api_base = Some(String::from("https://ctx.example.com"));
    chat.api_key = Some(key("ctx-key"));
    let params = r#"{"apikey":"param-key","url":"https://url.example.com","base_url":"https://legacy.example.com"}"#;
    chat.optional_params = object(params);
    let sent = text_call(&chat);
    assert_eq!(sent.custom_llm_provider, "watsonx_text");
    assert_eq!(sent.api_base.as_deref(), Some("https://ctx.example.com"));
    assert_eq!(exposed(sent.api_key.as_ref()).as_deref(), Some("ctx-key"));
    assert_eq!(sent.optional_params, object(params));
}

#[test]
fn watsonx_text_pops_the_parameter_keys_and_uses_them() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(
        r#"{"apikey":"param-key","base_url":"https://legacy.example.com","temperature":0.2}"#,
    );
    let sent = text_call(&chat);
    assert_eq!(sent.api_base.as_deref(), Some("https://legacy.example.com"));
    assert_eq!(exposed(sent.api_key.as_ref()).as_deref(), Some("param-key"));
    assert_eq!(sent.optional_params, object(r#"{"temperature":0.2}"#));
}

#[test]
fn watsonx_text_reads_the_environment_in_order() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.environment = BTreeMap::from([
        (String::from("WATSONX_APIKEY"), String::new()),
        (String::from("WATSONX_API_KEY"), String::from("second-key")),
        (String::from("WATSONX_API_BASE"), String::new()),
        (
            String::from("WATSONX_URL"),
            String::from("https://env.example.com"),
        ),
        (
            String::from("WX_URL"),
            String::from("https://wx-url.example.com"),
        ),
    ]);
    let sent = text_call(&chat);
    assert_eq!(
        exposed(sent.api_key.as_ref()).as_deref(),
        Some("second-key")
    );
    assert_eq!(sent.api_base.as_deref(), Some("https://env.example.com"));
}

#[test]
fn watsonx_text_prefers_url_and_removes_all_three_bases() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(
        r#"{"url":"https://url.example.com","api_base":"https://api-base.example.com","base_url":"https://base-url.example.com"}"#,
    );
    let sent = text_call(&chat);
    assert_eq!(sent.api_base.as_deref(), Some("https://url.example.com"));
    assert!(sent.optional_params.is_empty());
}

#[test]
fn watsonx_text_applies_the_wx_credentials_over_the_nested_ones() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(
        r#"{"watsonx_credentials":{"url":"https://nested.example.com","apikey":"nested-key","token":"nested-token"},"wx_credentials":{"url":"https://wx.example.com","api_key":"wx-key","watsonx_token":"wx-token"}}"#,
    );
    let sent = text_call(&chat);
    assert_eq!(sent.api_base.as_deref(), Some("https://wx.example.com"));
    assert_eq!(exposed(sent.api_key.as_ref()).as_deref(), Some("wx-key"));
    assert_eq!(sent.optional_params, object(r#"{"token":"wx-token"}"#));
}

#[test]
fn watsonx_text_reads_the_nested_credentials_alone() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(
        r#"{"watsonx_credentials":{"url":"https://nested.example.com","apikey":"nested-key","watsonx_token":"nested-token"}}"#,
    );
    let sent = text_call(&chat);
    assert_eq!(sent.api_base.as_deref(), Some("https://nested.example.com"));
    assert_eq!(
        exposed(sent.api_key.as_ref()).as_deref(),
        Some("nested-key")
    );
    assert_eq!(sent.optional_params, object(r#"{"token":"nested-token"}"#));
}

#[test]
fn watsonx_text_falls_to_the_nested_credentials_on_a_null_wx() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.api_base = Some(String::from("https://ctx.example.com"));
    chat.api_key = Some(key("ctx-key"));
    chat.optional_params = object(
        r#"{"wx_credentials":null,"watsonx_credentials":{"url":"https://nested.example.com","apikey":"nested-key"}}"#,
    );
    let sent = text_call(&chat);
    assert_eq!(sent.api_base.as_deref(), Some("https://nested.example.com"));
    assert_eq!(
        exposed(sent.api_key.as_ref()).as_deref(),
        Some("nested-key")
    );
    assert!(sent.optional_params.is_empty());
}

#[test]
fn watsonx_text_treats_a_null_url_as_unset() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(r#"{"url":null,"api_base":"https://api-base.example.com"}"#);
    let sent = text_call(&chat);
    assert_eq!(
        sent.api_base.as_deref(),
        Some("https://api-base.example.com")
    );
    assert!(sent.optional_params.is_empty());
}

#[test]
fn watsonx_text_credentials_keep_the_context_values_on_null() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.api_base = Some(String::from("https://ctx.example.com"));
    chat.api_key = Some(key("ctx-key"));
    chat.optional_params = object(r#"{"wx_credentials":{"url":null,"apikey":null}}"#);
    let sent = text_call(&chat);
    assert_eq!(sent.api_base.as_deref(), Some("https://ctx.example.com"));
    assert_eq!(exposed(sent.api_key.as_ref()).as_deref(), Some("ctx-key"));
    assert!(sent.optional_params.is_empty());
}

#[test]
fn watsonx_text_credentials_keep_the_context_key_when_absent() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.api_key = Some(key("ctx-key"));
    chat.optional_params = object(r#"{"wx_credentials":{"url":"https://wx.example.com"}}"#);
    let sent = text_call(&chat);
    assert_eq!(sent.api_base.as_deref(), Some("https://wx.example.com"));
    assert_eq!(exposed(sent.api_key.as_ref()).as_deref(), Some("ctx-key"));
}

#[test]
fn watsonx_text_credentials_fall_from_a_null_token() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(r#"{"wx_credentials":{"token":null,"watsonx_token":"nested"}}"#);
    let sent = text_call(&chat);
    assert_eq!(sent.optional_params, object(r#"{"token":"nested"}"#));
}

#[test]
fn watsonx_text_uses_an_explicit_empty_key_as_given() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.api_key = Some(key(""));
    chat.environment = BTreeMap::from([(String::from("WATSONX_APIKEY"), String::from("env-key"))]);
    let sent = text_call(&chat);
    assert_eq!(exposed(sent.api_key.as_ref()).as_deref(), Some(""));
}

#[test]
fn watsonx_text_uses_an_explicit_empty_base_as_given() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.api_base = Some(String::new());
    chat.environment = BTreeMap::from([(
        String::from("WATSONX_API_BASE"),
        String::from("https://env.example.com"),
    )]);
    let sent = text_call(&chat);
    assert_eq!(sent.api_base.as_deref(), Some(""));
}

#[test]
fn watsonx_text_rejects_a_typed_key() {
    let mut chat = Chat::new("watsonx_text", "ibm/granite");
    chat.optional_params = object(r#"{"apikey":5}"#);
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    let error = dispatch(ChatConfigTag::IbmWatsonXAi, &chat, &sagemaker, &recorder)
        .expect("the text arm runs")
        .expect_err("the key is not a string");
    assert!(matches!(
        error,
        EnterpriseError::FieldType { field: "apikey" }
    ));
    assert!(recorder.calls().is_empty());

    let mut nested = Chat::new("watsonx_text", "ibm/granite");
    nested.optional_params = object(r#"{"wx_credentials":{"url":5}}"#);
    let error = dispatch(ChatConfigTag::IbmWatsonXAi, &nested, &sagemaker, &recorder)
        .expect("the text arm runs")
        .expect_err("the url is not a string");
    assert!(matches!(error, EnterpriseError::FieldType { field: "url" }));
}

#[test]
fn sagemaker_chat_keeps_the_context_provider_name() {
    let mut chat = Chat::new("sagemaker_chat", "sagemaker_chat/endpoint");
    let recorder = Recorder::answer("reply");
    let sagemaker = Sagemaker::new(Scripted::empty());
    dispatch(ChatConfigTag::SagemakerChat, &chat, &sagemaker, &recorder)
        .expect("the sagemaker chat arm runs")
        .expect("the handler answers");
    assert_eq!(recorder.calls()[0].custom_llm_provider, "sagemaker_chat");

    chat.provider = "sagemaker_nova";
    chat.model = String::from("sagemaker_nova/endpoint");
    dispatch(ChatConfigTag::SagemakerNova, &chat, &sagemaker, &recorder)
        .expect("the nova arm runs")
        .expect("the handler answers");
    assert_eq!(recorder.calls()[1].custom_llm_provider, "sagemaker_nova");
}

#[tokio::test(flavor = "multi_thread")]
async fn sagemaker_completion_builds_the_signed_hf_call() {
    let chat = Chat::new("sagemaker", "sagemaker/endpoint");
    let sagemaker = Sagemaker::new(Scripted::new(vec![(200, r#"{"generated_text":"hello"}"#)]));
    let outcome = dispatch(
        ChatConfigTag::Sagemaker,
        &chat,
        &sagemaker,
        &Recorder::answer("unused"),
    )
    .expect("the sagemaker arm runs")
    .expect("the runtime answers");
    let Completion::Sagemaker(SagemakerReply::Completion(response)) = outcome else {
        panic!("a completion reply is expected");
    };
    assert_eq!(
        response.choices[0].message.content.as_deref(),
        Some("hello")
    );
    assert_eq!(response.model, "sagemaker/endpoint");
    assert_eq!(response.id, format!("chatcmpl-{}", identifier()));
    let sent = sagemaker.transport.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(
        sent[0].url,
        "https://runtime.sagemaker.us-west-2.amazonaws.com/endpoints/endpoint/invocations"
    );
    assert!(
        sagemaker
            .transport
            .header(0, http::header::AUTHORIZATION)
            .is_some_and(|value| value.starts_with("AWS4-HMAC-SHA256"))
    );
    assert_eq!(
        at(&sagemaker.transport.body(0), &["inputs"]),
        &Json::Text(String::from("hi"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sagemaker_completion_streams_when_the_flag_is_set() {
    let mut chat = Chat::new("sagemaker", "sagemaker/endpoint");
    chat.optional_params = object(r#"{"stream":true}"#);
    let sagemaker = Sagemaker::new(Scripted::empty());
    let outcome = dispatch(
        ChatConfigTag::Sagemaker,
        &chat,
        &sagemaker,
        &Recorder::answer("unused"),
    )
    .expect("the sagemaker arm runs");
    assert!(outcome.is_err());
    assert!(sagemaker.transport.streamed());
    let sent = sagemaker.transport.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].url.ends_with("/invocations-response-stream"));
}

#[tokio::test(flavor = "multi_thread")]
async fn sagemaker_completion_accepts_a_message_without_content() {
    let mut chat = Chat::new("sagemaker", "sagemaker/endpoint");
    chat.messages = vec![message(r#"{"role":"user"}"#)];
    chat.sagemaker_prompts = Some(BTreeMap::from([(
        String::from("sagemaker/endpoint"),
        CustomPromptEntry {
            roles: BTreeMap::new(),
            initial_prompt_value: String::from("Q:"),
            final_prompt_value: String::new(),
        },
    )]));
    let sagemaker = Sagemaker::new(Scripted::new(vec![(200, r#"{"generated_text":"hello"}"#)]));
    dispatch(
        ChatConfigTag::Sagemaker,
        &chat,
        &sagemaker,
        &Recorder::answer("unused"),
    )
    .expect("the sagemaker arm runs")
    .expect("the null content still builds a prompt");
    assert_eq!(
        at(&sagemaker.transport.body(0), &["inputs"]),
        &Json::Text(String::from("Q:"))
    );
}
