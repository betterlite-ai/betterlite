use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;

use crate::support;

#[tokio::test]
async fn async_compile_delegates_to_the_compile_helper() {
    let template = "---\nmodel: gpt-4\n---\nUser: Hello {{ name }}";
    let server = support::Server::start(&[("greeting.prompt", 200, template)]);
    let manager = BitBucketPromptManager::new(
        support::config(&server.base_url),
        None,
        support::transport(),
    );
    let variables: Object = serde_json::from_str(r#"{"name": "Ann"}"#).expect("variables");

    let sync_client = manager
        .compile_prompt_helper(Some("greeting"), Some(&variables), None, None)
        .await
        .expect("compile helper");
    let async_client = manager
        .async_compile_prompt_helper(Some("greeting"), Some(&variables), None, None)
        .await
        .expect("async compile helper");

    assert_eq!(async_client, sync_client);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn async_compile_requires_a_prompt_id() {
    let manager = BitBucketPromptManager::new(
        support::config(support::CLOSED_BASE),
        None,
        support::transport(),
    );

    let error = manager
        .async_compile_prompt_helper(None, None, None, None)
        .await
        .expect_err("prompt_id is required");

    assert_eq!(
        error.to_string(),
        "prompt_id is required for BitBucket prompt manager"
    );
}
