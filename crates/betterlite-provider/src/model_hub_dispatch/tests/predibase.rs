use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use serde_json::Number;

use super::super::{CustomPromptTable, HubCompletion, HubDispatchError, predibase};
use super::support::{
    NOW_SECONDS, Services, chat_context, empty_globals, environment, http_reply, object, run_with,
    user_message,
};
use crate::operation::OperationError;
use crate::predibase::error::PredibaseSetupError;

const MODEL: &str = "meta-llama/llama-2-7b-chat";
const BASE: Option<&str> = Some("http://mock");
const GENERATED: &str = r#"{"generated_text":"hello"}"#;
const TEMPLATE: &str = r#"{"roles":{"user":{"pre_message":"U:","post_message":"!"}},"initial_prompt_value":"I","final_prompt_value":"F"}"#;

#[tokio::test]
async fn the_predibase_arm_wraps_the_generate_reply() {
    let services = Services::new(Vec::new(), vec![http_reply(200, GENERATED)], Vec::new());
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let mut messages = vec![user_message()];
    let mut options = object(&[("tenant_id", PromptJson::Text(String::from("tenant-1")))]);
    let mut call = chat_context(MODEL, &mut messages, &mut options, BASE);
    let arm = (ChatConfigTag::Predibase, &mut call);
    let (outcome, state) = run_with(&services, &empty_globals(), &env, arm).await;
    let Some(HubCompletion::Predibase(completion)) = outcome.unwrap() else {
        panic!("the predibase tag returns a predibase completion");
    };
    let predibase::Completion::Reply(reply) = *completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.model, MODEL);
    assert_eq!(reply.created, NOW_SECONDS);
    let content = reply
        .choices
        .first()
        .and_then(|choice| choice.content.as_deref());
    assert_eq!(content, Some("hello"));
    let Some(Json::Object(args)) = state.model_call_details.get("additional_args") else {
        panic!("the post_call keeps its arguments");
    };
    assert!(args.contains_key("complete_input_dict"));
    let Some(Json::Object(raw)) = state.model_call_details.get("raw_request_typed_dict") else {
        panic!("the pre_call records the raw request");
    };
    let Some(Json::Object(headers)) = raw.get("raw_request_headers") else {
        panic!("the raw request keeps its headers");
    };
    assert!(headers.contains_key("content-type"));
}

#[tokio::test]
async fn the_predibase_arm_sends_the_generate_request() {
    let services = Services::new(Vec::new(), vec![http_reply(200, GENERATED)], Vec::new());
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let mut messages = vec![user_message()];
    let mut options = object(&[("tenant_id", PromptJson::Text(String::from("tenant-1")))]);
    let mut call = chat_context(MODEL, &mut messages, &mut options, BASE);
    call.acompletion = true;
    let arm = (ChatConfigTag::Predibase, &mut call);
    let (outcome, _logged) = run_with(&services, &empty_globals(), &env, arm).await;
    outcome.unwrap();
    let posted = services.http.sent("POST");
    let [sent] = posted.as_slice() else {
        panic!("the predibase arm posts once");
    };
    let url = "http://mock/tenant-1/deployments/v2/llms/meta-llama/llama-2-7b-chat/generate";
    assert_eq!(sent.url, url);
    assert_eq!(sent.headers.get("Authorization").unwrap(), "Bearer pb-env");
    let body: Object = serde_json::from_str(&sent.body).unwrap();
    let inputs = PromptJson::Text(String::from("<s>[INST] hi [/INST]\n"));
    assert_eq!(body.get("inputs"), Some(&inputs));
    assert!(!body.contains_key("stream"));
    let parameters = PromptJson::Object(object(&[
        ("details", PromptJson::Bool(true)),
        ("max_new_tokens", PromptJson::Number(Number::from(4096_i64))),
        ("return_full_text", PromptJson::Bool(false)),
    ]));
    assert_eq!(body.get("parameters"), Some(&parameters));
}

#[tokio::test]
async fn the_predibase_arm_fails_with_the_tenant_id_text() {
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, BASE);
    let arm = (ChatConfigTag::Predibase, &mut call);
    let (outcome, _logged) = run_with(&services, &empty_globals(), &env, arm).await;
    let Err(error) = outcome else {
        panic!("a missing tenant id fails the call");
    };
    let HubDispatchError::PredibaseSetup(PredibaseSetupError::MissingTenantId) = &error else {
        panic!("the missing tenant id is a setup error");
    };
    assert_eq!(
        error.to_string(),
        "Missing Predibase Tenant ID - Required for making the request. Set dynamically (e.g. `completion(..tenant_id=<MY-ID>)`) or in env - `PREDIBASE_TENANT_ID`."
    );
    assert!(services.http.sent("POST").is_empty());
}

#[tokio::test]
async fn the_predibase_arm_streams_with_the_stream_suffix_and_body_flag() {
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let mut messages = vec![user_message()];
    let mut options = object(&[("tenant_id", PromptJson::Text(String::from("tenant-1")))]);
    options.insert(String::from("stream"), PromptJson::Bool(true));
    let mut call = chat_context("m", &mut messages, &mut options, BASE);
    call.acompletion = true;
    let arm = (ChatConfigTag::Predibase, &mut call);
    let (outcome, _logged) = run_with(&services, &empty_globals(), &env, arm).await;
    let Err(HubDispatchError::Operation(OperationError::UnsupportedOperation { .. })) = outcome
    else {
        panic!("the scripted transport refuses the stream");
    };
    let posted = services.http.sent("POST");
    let [sent] = posted.as_slice() else {
        panic!("the arm opens one stream");
    };
    assert_eq!(
        sent.url,
        "http://mock/tenant-1/deployments/v2/llms/m/generate_stream"
    );
    let body: Object = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(body.get("stream"), Some(&PromptJson::Bool(true)));
    let Some(PromptJson::Object(parameters)) = body.get("parameters") else {
        panic!("the request keeps its parameters");
    };
    assert!(!parameters.contains_key("stream"));
}

#[tokio::test]
async fn the_predibase_arm_builds_the_prompt_from_the_custom_prompt_dict() {
    let services = Services::new(Vec::new(), vec![http_reply(200, GENERATED)], Vec::new());
    let template: CustomPromptTemplate = serde_json::from_str(TEMPLATE).unwrap();
    let table = CustomPromptTable::from([(String::from(MODEL), template)]);
    let env = environment(&[("PREDIBASE_API_KEY", "pb-env")]);
    let mut messages = vec![user_message()];
    let mut options = object(&[("tenant_id", PromptJson::Text(String::from("tenant-1")))]);
    let mut call = chat_context(MODEL, &mut messages, &mut options, BASE);
    call.custom_prompt_dict = Some(&table);
    let arm = (ChatConfigTag::Predibase, &mut call);
    let (outcome, _logged) = run_with(&services, &empty_globals(), &env, arm).await;
    outcome.unwrap();
    let posted = services.http.sent("POST");
    let [sent] = posted.as_slice() else {
        panic!("the predibase arm posts once");
    };
    let body: Object = serde_json::from_str(&sent.body).unwrap();
    let inputs = PromptJson::Text(String::from("IU:hi!F"));
    assert_eq!(body.get("inputs"), Some(&inputs));
}
