use betterlite_models::decisions::Json;

use super::support;

#[tokio::test]
async fn a_missing_prompt_id_never_runs() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    assert!(
        !fixture
            .manager
            .should_run_prompt_management(None, None, &dynamic)
            .await
    );
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
    assert!(
        fixture
            .manager
            .should_run_prompt_management(Some("greeting"), None, &dynamic)
            .await
    );
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
    assert!(
        !fixture
            .manager
            .should_run_prompt_management(Some("missing"), None, &dynamic)
            .await
    );
}

#[tokio::test]
async fn dynamic_params_that_are_not_an_object_do_not_run() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    assert!(
        !fixture
            .manager
            .should_run_prompt_management(Some("greeting"), None, &Json::Text("oops".to_owned()))
            .await
    );
}
