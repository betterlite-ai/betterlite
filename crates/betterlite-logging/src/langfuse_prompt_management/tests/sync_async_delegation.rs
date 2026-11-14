use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{PromptIgnoreFlags, PromptParams};
use indexmap::IndexMap;

use super::support;
use crate::langfuse_prompt_management::completion::{
    AsyncChatCompletionPromptRequest, ChatCompletionPromptRequest,
};

fn message() -> Json {
    Json::Object(IndexMap::from([(
        "role".to_owned(),
        Json::Text("user".to_owned()),
    )]))
}

#[tokio::test]
async fn without_a_prompt_id_the_async_hook_returns_the_callers_request() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    let messages = vec![message()];
    let request = ChatCompletionPromptRequest {
        model: "gpt-4o",
        messages: messages.clone(),
        non_default_params: PromptParams::new(),
        prompt_id: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_spec: None,
        prompt_label: None,
        prompt_version: None,
        ignore_flags: PromptIgnoreFlags::default(),
    };
    let result = fixture
        .manager
        .async_get_chat_completion_prompt(AsyncChatCompletionPromptRequest {
            request,
            tools: Some(&[]),
        })
        .await
        .expect("the request passes through");
    assert_eq!(result.model, "gpt-4o");
    assert_eq!(result.messages, messages);
    assert!(result.non_default_params.is_empty());
}

#[tokio::test]
async fn the_async_hook_answers_exactly_like_the_sync_base() {
    let server = support::FakeLangfuse::start(vec![
        (200, support::PROJECTS.to_owned()),
        (200, support::chat_prompt()),
    ])
    .await;
    let variables = support::env(&server.base_url);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    let prompt_variables: PromptParams =
        serde_json::from_str(r#"{"name":"Ada","history":[{"role":"user","content":"hi"}]}"#)
            .expect("the variables parse");
    let request = || ChatCompletionPromptRequest {
        model: "gpt-4o-mini",
        messages: vec![message()],
        non_default_params: PromptParams::new(),
        prompt_id: Some("greeting"),
        prompt_variables: Some(&prompt_variables),
        dynamic_callback_params: &dynamic,
        prompt_spec: None,
        prompt_label: None,
        prompt_version: None,
        ignore_flags: PromptIgnoreFlags::default(),
    };
    let synchronous = fixture
        .manager
        .get_chat_completion_prompt(request())
        .await
        .expect("the sync base compiles the prompt");
    let asynchronous = fixture
        .manager
        .async_get_chat_completion_prompt(AsyncChatCompletionPromptRequest {
            request: request(),
            tools: None,
        })
        .await
        .expect("the async override delegates");
    assert_eq!(asynchronous, synchronous);
    assert_eq!(asynchronous.model, "gpt-4o");
    assert_eq!(asynchronous.messages.len(), 3);
    assert!(asynchronous.non_default_params.contains_key("temperature"));
}
