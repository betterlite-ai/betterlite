#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{PromptManagementClient, PromptParams};

use super::{CompileError, PhoenixPromptManager};
use crate::arize::phoenix::caching::TemplateCache;
use crate::arize::phoenix::rendering::sandbox;
use crate::arize::phoenix::tests::server::{PROMPT_BODY, phoenix_client, serve};

fn manager(base: &str, prompt_id: Option<&str>) -> PhoenixPromptManager {
    PhoenixPromptManager {
        client: phoenix_client(base),
        prompt_id: prompt_id.map(str::to_owned),
        templates: TemplateCache::default(),
        environment: sandbox(),
    }
}

fn variables(text: &str) -> PromptParams {
    serde_json::from_str(text).unwrap()
}

fn messages(text: &str) -> Vec<Json> {
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn integration_compiles_rendered_messages_with_model_and_optional_parameters() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let compiled = manager(&phoenix.base, None)
        .compile_prompt_helper(
            Some("UHJvbXB0VmVyc2lvbjox"),
            Some(&variables(r#"{"topic":"math","question":"2+2?"}"#)),
        )
        .await
        .unwrap();
    assert_eq!(
        compiled,
        PromptManagementClient {
            prompt_id: Some("UHJvbXB0VmVyc2lvbjox".to_owned()),
            prompt_template: messages(
                r#"[{"role":"system","content":"You answer questions about math."},{"role":"user","content":"2+2?"}]"#
            ),
            prompt_template_model: Some("gpt-4o".to_owned()),
            prompt_optional_params: Some(variables(
                r#"{"temperature":0.5,"max_tokens":256,"top_p":0.9}"#
            )),
            completed_messages: None,
        }
    );
}

#[tokio::test]
async fn integration_escapes_variables_inside_the_compiled_messages() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let compiled = manager(&phoenix.base, None)
        .compile_prompt_helper(
            Some("v1"),
            Some(&variables(r#"{"topic":"<script>","question":"a & b"}"#)),
        )
        .await
        .unwrap();
    assert_eq!(
        compiled.prompt_template,
        messages(
            r#"[{"role":"system","content":"You answer questions about &lt;script&gt;."},{"role":"user","content":"a &amp; b"}]"#
        )
    );
}

#[tokio::test]
async fn integration_without_variables_renders_empty_placeholders() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let compiled = manager(&phoenix.base, None)
        .compile_prompt_helper(Some("v1"), None)
        .await
        .unwrap();
    assert_eq!(
        compiled.prompt_template,
        messages(
            r#"[{"role":"system","content":"You answer questions about ."},{"role":"user","content":""}]"#
        )
    );
}

#[tokio::test]
async fn integration_reuses_the_cached_template_for_later_compilations() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let manager = manager(&phoenix.base, None);
    let first = manager
        .compile_prompt_helper(Some("v1"), Some(&variables(r#"{"question":"one"}"#)))
        .await
        .unwrap();
    let second = manager
        .compile_prompt_helper(Some("v1"), Some(&variables(r#"{"question":"two"}"#)))
        .await
        .unwrap();
    assert_ne!(first.prompt_template, second.prompt_template);
    assert_eq!(phoenix.request_heads().len(), 1);
}

#[tokio::test]
async fn integration_requires_a_prompt_id() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let error = manager(&phoenix.base, None)
        .compile_prompt_helper(None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, CompileError::PromptIdRequired));
    assert_eq!(
        error.to_string(),
        "prompt_id is required for Arize Phoenix prompt manager"
    );
    assert!(phoenix.request_heads().is_empty());
}

#[tokio::test]
async fn integration_names_the_prompt_when_the_version_is_missing() {
    let phoenix = serve(404, "{}").await;
    let error = manager(&phoenix.base, None)
        .compile_prompt_helper(Some("v1"), None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error compiling prompt 'v1': Failed to load prompt version 'v1' from Arize Phoenix: Prompt version 'v1' not found"
    );
}

#[tokio::test]
async fn integration_reports_access_denied_through_the_compile_error() {
    let phoenix = serve(403, "{}").await;
    let error = manager(&phoenix.base, None)
        .compile_prompt_helper(Some("v1"), None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error compiling prompt 'v1': Failed to load prompt version 'v1' from Arize Phoenix: Access denied to prompt version 'v1'. Check your Arize Phoenix permissions."
    );
}

#[tokio::test]
async fn integration_reports_a_rejected_identifier_through_the_compile_error() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let error = manager(&phoenix.base, None)
        .compile_prompt_helper(Some("a/b"), None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error compiling prompt 'a/b': Failed to load prompt version 'a/b' from Arize Phoenix: Invalid identifier 'a/b': contains disallowed characters"
    );
    assert!(phoenix.request_heads().is_empty());
}

#[tokio::test]
async fn integration_reports_a_template_that_cannot_render() {
    let phoenix = serve(
        200,
        r#"{"data":{"template":{"messages":[{"role":"user","content":[{"type":"text","text":"{% if %}"}]}]}}}"#,
    )
    .await;
    let error = manager(&phoenix.base, None)
        .compile_prompt_helper(Some("v1"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, CompileError::Render { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Error compiling prompt 'v1': ")
    );
}

#[tokio::test]
async fn integration_loads_the_configured_prompt_before_the_requested_one() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let manager = manager(&phoenix.base, Some("configured"));
    manager
        .compile_prompt_helper(Some("other"), None)
        .await
        .unwrap();
    manager
        .compile_prompt_helper(Some("configured"), None)
        .await
        .unwrap();
    let heads = phoenix.request_heads();
    assert_eq!(heads.len(), 2);
    assert!(heads[0].starts_with("GET /v1/prompt_versions/configured "));
    assert!(heads[1].starts_with("GET /v1/prompt_versions/other "));
}

#[tokio::test]
async fn integration_fails_a_compilation_when_the_configured_prompt_cannot_load() {
    let phoenix = serve(404, "{}").await;
    let error = manager(&phoenix.base, Some("configured"))
        .compile_prompt_helper(Some("other"), None)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error compiling prompt 'other': Failed to load prompt version 'configured' from Arize Phoenix: Prompt version 'configured' not found"
    );
}

#[test]
fn integration_name_is_arize() {
    assert_eq!(PhoenixPromptManager::INTEGRATION_NAME, "arize");
}
