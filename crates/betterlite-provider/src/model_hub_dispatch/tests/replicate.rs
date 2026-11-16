use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use secrecy::SecretString;

use super::super::{CustomPromptTable, HubCompletion, HubDispatchError};
use super::support::{
    NOW_SECONDS, Services, chat_context, empty_globals, environment, object, replicate_reply,
    run_with, user_message,
};
use crate::replicate::poll::ReplicateCompletion;

const MODEL: &str = "meta-llama/llama-2-70b-chat";
const CREATE: &str = r#"{"urls":{"get":"https://poll/1"}}"#;
const DONE: &str = r#"{"status":"succeeded","output":["ok"]}"#;
const PROMPT: &str = r#"{"input":{"prompt":"<s>[INST] hi [/INST]\n"}}"#;
const TEMPLATE: &str = r#"{"roles":{"user":{"pre_message":"U:","post_message":"!"}},"initial_prompt_value":"I","final_prompt_value":"F"}"#;

fn polled() -> Services {
    Services::new(
        Vec::new(),
        Vec::new(),
        vec![replicate_reply(CREATE), replicate_reply(DONE)],
    )
}

// the prompt the create request carried
fn created_prompt(services: &Services) -> Option<PromptJson> {
    let sent = services.transport.sent.lock().unwrap();
    let create = sent.first()?;
    let body: Object = serde_json::from_slice(create.body.as_deref()?).ok()?;
    let Some(PromptJson::Object(input)) = body.get("input") else {
        return None;
    };
    input.get("prompt").cloned()
}

#[tokio::test]
async fn the_replicate_arm_uses_the_strict_policy_for_a_sync_call() {
    let services = polled();
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None);
    let arm = (ChatConfigTag::Replicate, &mut call);
    let (outcome, state) = run_with(&services, &empty_globals(), &environment(&[]), arm).await;
    let Some(HubCompletion::Replicate(completion)) = outcome.unwrap() else {
        panic!("the replicate tag returns a replicate completion");
    };
    let ReplicateCompletion::Reply(reply) = *completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.text, "ok");
    assert_eq!(reply.model, "replicate/meta-llama/llama-2-70b-chat");
    assert_eq!(reply.created, Some(NOW_SECONDS));
    assert_eq!(reply.usage.prompt_tokens, 1);
    assert!(state.model_call_details.get("log_event_type").is_none());
}

#[tokio::test]
async fn the_replicate_arm_creates_then_polls_with_the_context_key() {
    let services = polled();
    let key = SecretString::from("ctx-key");
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None);
    call.api_key = Some(&key);
    let arm = (ChatConfigTag::Replicate, &mut call);
    let (outcome, _logged) = run_with(&services, &empty_globals(), &environment(&[]), arm).await;
    outcome.unwrap();
    let sent = services.transport.sent.lock().unwrap().clone();
    let [create, poll] = sent.as_slice() else {
        panic!("the arm creates the prediction and polls it once");
    };
    let url = "https://api.replicate.com/v1/models/meta-llama/llama-2-70b-chat/predictions";
    assert_eq!(create.url, url);
    assert_eq!(
        create.headers.get("authorization").unwrap(),
        "Token ctx-key"
    );
    let body: Object = serde_json::from_slice(create.body.as_deref().unwrap()).unwrap();
    assert_eq!(body, serde_json::from_str::<Object>(PROMPT).unwrap());
    assert_eq!(poll.url, "https://poll/1");
    let delays = services.sleeper.delays.lock().unwrap().clone();
    let bits: Vec<u64> = delays.iter().map(|delay| delay.to_bits()).collect();
    assert_eq!(bits, vec![1.0_f64.to_bits()]);
}

#[tokio::test]
async fn the_replicate_arm_uses_the_lenient_policy_for_an_async_call() {
    let lenient = r#"{"status":"succeeded","output":5}"#;
    let replies = vec![replicate_reply(CREATE), replicate_reply(lenient)];
    let services = Services::new(Vec::new(), Vec::new(), replies);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None);
    call.acompletion = true;
    let arm = (ChatConfigTag::Replicate, &mut call);
    let (outcome, _logged) = run_with(&services, &empty_globals(), &environment(&[]), arm).await;
    let Some(HubCompletion::Replicate(completion)) = outcome.unwrap() else {
        panic!("the replicate tag returns a replicate completion");
    };
    let ReplicateCompletion::Reply(reply) = *completion else {
        panic!("a non-stream call returns the reply");
    };
    assert_eq!(reply.text, "5");
    assert_eq!(reply.created, None);
}

#[tokio::test]
async fn the_replicate_arm_reports_a_missing_prediction_url() {
    let services = Services::new(Vec::new(), Vec::new(), vec![replicate_reply("{}")]);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None);
    let arm = (ChatConfigTag::Replicate, &mut call);
    let (outcome, _logged) = run_with(&services, &empty_globals(), &environment(&[]), arm).await;
    let Err(HubDispatchError::Replicate(failure)) = outcome else {
        panic!("a create reply without a poll url is a replicate failure");
    };
    let Some(api) = failure.api() else {
        panic!("the missing url is a Replicate API error");
    };
    assert_eq!(api.status_code, 400);
    assert!(
        api.message
            .starts_with("LiteLLM Error - prediction url is None - ")
    );
}

#[tokio::test]
async fn the_replicate_arm_logs_the_post_call_for_a_stream() {
    let services = Services::new(Vec::new(), Vec::new(), vec![replicate_reply(CREATE)]);
    let mut messages = vec![user_message()];
    let mut options = object(&[("stream", PromptJson::Bool(true))]);
    let mut call = chat_context(MODEL, &mut messages, &mut options, None);
    let arm = (ChatConfigTag::Replicate, &mut call);
    let (outcome, state) = run_with(&services, &empty_globals(), &environment(&[]), arm).await;
    let Some(HubCompletion::Replicate(completion)) = outcome.unwrap() else {
        panic!("the replicate tag returns a replicate completion");
    };
    assert!(matches!(*completion, ReplicateCompletion::Stream(_)));
    let event = state.model_call_details.get("log_event_type");
    assert_eq!(event, Some(&Json::Text(String::from("post_api_call"))));
    let Some(Json::List(input)) = state.model_call_details.get("input") else {
        panic!("the post_call keeps the messages");
    };
    assert_eq!(input.len(), 1);
}

#[tokio::test]
async fn the_replicate_arm_renders_the_call_custom_prompt() {
    let services = polled();
    let template: CustomPromptTemplate = serde_json::from_str(TEMPLATE).unwrap();
    let table = CustomPromptTable::from([(String::from(MODEL), template)]);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None);
    call.custom_prompt_dict = Some(&table);
    let arm = (ChatConfigTag::Replicate, &mut call);
    let (outcome, _logged) = run_with(&services, &empty_globals(), &environment(&[]), arm).await;
    outcome.unwrap();
    let rendered = PromptJson::Text(String::from("IU:hi!F"));
    assert_eq!(created_prompt(&services), Some(rendered));
}

#[tokio::test]
async fn the_replicate_arm_falls_back_to_the_global_custom_prompt() {
    let services = polled();
    let template: CustomPromptTemplate = serde_json::from_str(TEMPLATE).unwrap();
    let table = CustomPromptTable::from([(String::from(MODEL), template)]);
    let mut globals = empty_globals();
    globals.custom_prompt_dict = Some(&table);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, None);
    let arm = (ChatConfigTag::Replicate, &mut call);
    let (outcome, _logged) = run_with(&services, &globals, &environment(&[]), arm).await;
    outcome.unwrap();
    let rendered = PromptJson::Text(String::from("IU:hi!F"));
    assert_eq!(created_prompt(&services), Some(rendered));
}
