use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;
use tokio::sync::OnceCell;

use crate::support;

#[tokio::test]
async fn compile_exception_wrap_names_the_prompt_and_the_cause() {
    let server = support::Server::start(&[("greeting.prompt", 500, "boom")]);
    let manager = BitBucketPromptManager {
        bitbucket_config: support::config(&server.base_url),
        prompt_id: None,
        transport: support::transport(),
        template_manager: OnceCell::const_new(),
    };

    let error = manager
        .compile_prompt_helper(Some("greeting"), None, None, None)
        .await
        .expect_err("the fetch answers with an error");

    assert_eq!(
        error.to_string(),
        "Error compiling prompt 'greeting': Failed to load prompt 'greeting' from BitBucket: \
         Failed to fetch file 'greeting.prompt': BitBucket answered with status 500"
    );
}
