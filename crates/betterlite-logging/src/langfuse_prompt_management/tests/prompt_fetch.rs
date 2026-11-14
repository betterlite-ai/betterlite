use betterlite_models::prompt_contracts::PromptCompileRequest;

use super::support;

#[tokio::test]
async fn the_prompt_is_fetched_with_its_label_and_version() {
    let server = support::FakeLangfuse::start(vec![
        (200, support::PROJECTS.to_owned()),
        (200, support::chat_prompt()),
    ])
    .await;
    let variables = support::env(&server.base_url);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    let request = PromptCompileRequest {
        prompt_id: Some("greeting"),
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: Some("production"),
        prompt_version: Some(3),
    };
    let client = fixture
        .manager
        .compile_prompt_helper(&request)
        .await
        .expect("the prompt compiles");
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /api/public/projects"));
    assert!(
        requests[1].starts_with("GET /api/public/v2/prompts/greeting?version=3&label=production"),
        "the fetch used {}",
        requests[1]
    );
    assert_eq!(client.prompt_id.as_deref(), Some("greeting"));
}

#[tokio::test]
async fn the_prompt_is_fetched_without_a_query_when_no_label_or_version_is_given() {
    let server = support::FakeLangfuse::start(vec![
        (200, support::PROJECTS.to_owned()),
        (200, support::text_prompt()),
    ])
    .await;
    let variables = support::env(&server.base_url);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    let request = PromptCompileRequest {
        prompt_id: Some("intro"),
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: None,
        prompt_version: None,
    };
    fixture
        .manager
        .compile_prompt_helper(&request)
        .await
        .expect("the prompt compiles");
    let requests = server.requests();
    assert!(
        requests[1].starts_with("GET /api/public/v2/prompts/intro HTTP/1.1"),
        "the fetch used {}",
        requests[1]
    );
}
