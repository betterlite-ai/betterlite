use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::PromptCompileRequest;

use super::support;
use crate::langfuse_prompt_management::LangfusePromptManagementError;

#[tokio::test]
async fn the_helper_returns_the_compiled_prompt_model_and_optional_params() {
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
    let request = PromptCompileRequest {
        prompt_id: Some("greeting"),
        prompt_spec: None,
        prompt_variables: Some(&prompt_variables),
        dynamic_callback_params: &dynamic,
        prompt_label: None,
        prompt_version: None,
    };
    let client = fixture
        .manager
        .compile_prompt_helper(&request)
        .await
        .expect("the prompt compiles");
    assert_eq!(client.prompt_id.as_deref(), Some("greeting"));
    assert_eq!(client.prompt_template.len(), 2);
    assert_eq!(client.prompt_template_model.as_deref(), Some("gpt-4o"));
    let optional = client
        .prompt_optional_params
        .expect("the optional params are present");
    assert!(optional.contains_key("temperature"));
    assert_eq!(client.completed_messages, None);
    assert_eq!(
        client.prompt_template[1],
        Json::Object(indexmap::IndexMap::from([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), Json::Text("hi".to_owned())),
        ]))
    );
}

#[tokio::test]
async fn a_missing_prompt_id_is_reported() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let dynamic = support::no_dynamic_params();
    let request = PromptCompileRequest {
        prompt_id: None,
        prompt_spec: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_label: None,
        prompt_version: None,
    };
    let error = fixture
        .manager
        .compile_prompt_helper(&request)
        .await
        .expect_err("a prompt id is required");
    assert!(matches!(
        error,
        LangfusePromptManagementError::MissingPromptId
    ));
    assert_eq!(
        error.to_string(),
        "prompt_id is required for Langfuse prompt management"
    );
}
