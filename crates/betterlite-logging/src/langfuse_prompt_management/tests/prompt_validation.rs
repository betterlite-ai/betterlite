use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{PromptIgnoreFlags, PromptParams};
use indexmap::IndexMap;

use super::support;
use crate::langfuse_prompt_management::compilation::should_run_prompt_management;
use crate::langfuse_prompt_management::completion::{
    ChatCompletionPromptRequest, get_chat_completion_prompt,
};

#[tokio::test]
async fn a_missing_prompt_id_never_runs() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    assert!(!should_run_prompt_management(&fixture.manager, None, None, &dynamic).await);
}

#[tokio::test]
async fn a_prompt_that_fetches_runs() {
    let server = support::FakeLangfuse::start(vec![
        (200, support::PROJECTS.to_owned()),
        (200, support::chat_prompt()),
    ])
    .await;
    let variables = support::env(&server.base_url);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    assert!(should_run_prompt_management(&fixture.manager, Some("greeting"), None, &dynamic).await);
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn a_failed_fetch_does_not_run_and_does_not_propagate() {
    let server = support::FakeLangfuse::start(vec![
        (200, support::PROJECTS.to_owned()),
        (404, "{}".to_owned()),
    ])
    .await;
    let variables = support::env(&server.base_url);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    assert!(!should_run_prompt_management(&fixture.manager, Some("missing"), None, &dynamic).await);
}

#[tokio::test]
async fn a_failing_fetch_returns_the_callers_request_unchanged() {
    let server = support::FakeLangfuse::start(vec![
        (200, support::PROJECTS.to_owned()),
        (404, "{}".to_owned()),
    ])
    .await;
    let variables = support::env(&server.base_url);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    let messages = vec![Json::Object(IndexMap::from([(
        "role".to_owned(),
        Json::Text("user".to_owned()),
    )]))];
    let result = get_chat_completion_prompt(
        &fixture.manager,
        ChatCompletionPromptRequest {
            model: "gpt-4o-mini",
            messages: messages.clone(),
            non_default_params: PromptParams::new(),
            prompt_id: Some("missing"),
            prompt_variables: None,
            dynamic_callback_params: &dynamic,
            prompt_spec: None,
            prompt_label: None,
            prompt_version: None,
            ignore_flags: PromptIgnoreFlags::default(),
        },
    )
    .await
    .expect("the request passes through");
    assert_eq!(result.model, "gpt-4o-mini");
    assert_eq!(result.messages, messages);
    assert!(result.non_default_params.is_empty());
}

#[tokio::test]
async fn dynamic_params_that_are_not_an_object_do_not_run() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    assert!(
        !should_run_prompt_management(
            &fixture.manager,
            Some("greeting"),
            None,
            &Json::Text("oops".to_owned())
        )
        .await
    );
}
