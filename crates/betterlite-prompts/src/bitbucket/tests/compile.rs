use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;
use tokio::sync::OnceCell;

use betterlite_prompts::arguments::json::Object;

use crate::support;

#[tokio::test]
async fn compile_loads_the_missing_prompt_and_parses_the_rendered_messages() {
    let template = "---\nmodel: gpt-4\n---\nSystem: You greet\nUser: Hello {{ name }}";
    let server = support::Server::start(&[("greeting.prompt", 200, template)]);
    let manager = BitBucketPromptManager {
        bitbucket_config: support::config(&server.base_url),
        prompt_id: None,
        transport: support::transport(),
        template_manager: OnceCell::const_new(),
    };
    let variables: Object = serde_json::from_str(r#"{"name": "Ann"}"#).expect("variables");

    let client = manager
        .compile_prompt_helper(Some("greeting"), Some(&variables), None, None)
        .await
        .expect("compiled prompt");

    assert_eq!(client.prompt_id.as_deref(), Some("greeting"));
    assert_eq!(client.prompt_template_model.as_deref(), Some("gpt-4"));
    assert!(client.completed_messages.is_none());
    assert_eq!(
        client.prompt_template,
        support::messages(&[("system", "You greet"), ("user", "Hello Ann")])
    );
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn compile_requires_a_prompt_id() {
    let manager = BitBucketPromptManager {
        bitbucket_config: support::config(support::CLOSED_BASE),
        prompt_id: None,
        transport: support::transport(),
        template_manager: OnceCell::const_new(),
    };

    let error = manager
        .compile_prompt_helper(None, None, None, None)
        .await
        .expect_err("prompt_id is required");

    assert_eq!(
        error.to_string(),
        "prompt_id is required for BitBucket prompt manager"
    );
}
