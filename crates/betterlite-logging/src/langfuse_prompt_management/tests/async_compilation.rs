use betterlite_models::prompt_contracts::{PromptAsyncCompileRequest, PromptCompileRequest};

use super::support;

#[tokio::test]
async fn the_async_helper_answers_exactly_like_the_sync_one() {
    let server = support::FakeLangfuse::start(vec![
        (200, support::PROJECTS.to_owned()),
        (200, support::chat_prompt()),
    ])
    .await;
    let variables = support::env(&server.base_url);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    let prompt_variables: betterlite_models::prompt_contracts::PromptParams =
        serde_json::from_str(r#"{"name":"Ada","history":[{"role":"user","content":"hi"}]}"#)
            .expect("the variables parse");
    let async_request = PromptAsyncCompileRequest {
        prompt_id: Some("greeting"),
        prompt_variables: Some(&prompt_variables),
        dynamic_callback_params: &dynamic,
        prompt_spec: None,
        prompt_label: Some("production"),
        prompt_version: Some(3),
    };
    let asynchronous = fixture
        .manager
        .async_compile_prompt_helper(&async_request)
        .await
        .expect("the async helper compiles");
    let sync_request = PromptCompileRequest {
        prompt_id: Some("greeting"),
        prompt_spec: None,
        prompt_variables: Some(&prompt_variables),
        dynamic_callback_params: &dynamic,
        prompt_label: Some("production"),
        prompt_version: Some(3),
    };
    let synchronous = fixture
        .manager
        .compile_prompt_helper(&sync_request)
        .await
        .expect("the sync helper compiles");
    assert_eq!(asynchronous, synchronous);
}
