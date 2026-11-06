#![cfg(test)]

use std::sync::Arc;

use super::{LoadError, TemplateCache};
use crate::arize::phoenix::tests::server::{PROMPT_BODY, phoenix_client, serve};

#[tokio::test]
async fn caching_a_second_load_reuses_the_template_without_a_request() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let client = phoenix_client(&phoenix.base);
    let cache = TemplateCache::default();
    let first = cache.load(&client, "v1").await.unwrap();
    let second = cache.load(&client, "v1").await.unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.template_id, "v1");
    assert_eq!(phoenix.request_heads().len(), 1);
}

#[tokio::test]
async fn caching_keys_templates_by_prompt_version_id() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let client = phoenix_client(&phoenix.base);
    let cache = TemplateCache::default();
    let one = cache.load(&client, "v1").await.unwrap();
    let two = cache.load(&client, "v2").await.unwrap();
    cache.load(&client, "v1").await.unwrap();
    assert_eq!(
        (one.template_id.as_str(), two.template_id.as_str()),
        ("v1", "v2")
    );
    assert_eq!(phoenix.request_heads().len(), 2);
}

#[tokio::test]
async fn caching_does_not_keep_a_version_phoenix_could_not_find() {
    let phoenix = serve(404, "{}").await;
    let client = phoenix_client(&phoenix.base);
    let cache = TemplateCache::default();
    let error = cache.load(&client, "v1").await.unwrap_err();
    cache.load(&client, "v1").await.unwrap_err();
    assert!(matches!(error, LoadError::NotFound { .. }));
    assert_eq!(
        error.to_string(),
        "Failed to load prompt version 'v1' from Arize Phoenix: Prompt version 'v1' not found"
    );
    assert_eq!(phoenix.request_heads().len(), 2);
}

#[tokio::test]
async fn caching_wraps_a_client_failure_with_the_version_it_was_loading() {
    let phoenix = serve(403, "{}").await;
    let error = TemplateCache::default()
        .load(&phoenix_client(&phoenix.base), "v1")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to load prompt version 'v1' from Arize Phoenix: Access denied to prompt version 'v1'. Check your Arize Phoenix permissions."
    );
}

#[tokio::test]
async fn caching_wraps_a_parsing_failure_and_stores_nothing() {
    let phoenix = serve(
        200,
        r#"{"data":{"invocation_parameters":{"openai":{"temperature":"hot"}}}}"#,
    )
    .await;
    let client = phoenix_client(&phoenix.base);
    let cache = TemplateCache::default();
    let error = cache.load(&client, "v1").await.unwrap_err();
    cache.load(&client, "v1").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to load prompt version 'v1' from Arize Phoenix: invocation_parameters temperature must be a number"
    );
    assert_eq!(phoenix.request_heads().len(), 2);
}

#[tokio::test]
async fn caching_rejects_an_unsafe_identifier_without_a_request() {
    let phoenix = serve(200, PROMPT_BODY).await;
    let error = TemplateCache::default()
        .load(&phoenix_client(&phoenix.base), "../x")
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Failed to load prompt version '../x' from Arize Phoenix: Invalid identifier '../x': contains disallowed characters"
    );
    assert!(phoenix.request_heads().is_empty());
}
