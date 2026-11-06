use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;
use tokio::sync::OnceCell;

use crate::support;

#[tokio::test]
async fn get_template_not_found_names_the_prompt() {
    let server = support::Server::start(&[("greeting.prompt", 200, "Hi")]);
    let manager = BitBucketPromptManager {
        bitbucket_config: support::config(&server.base_url),
        prompt_id: Some("greeting".to_owned()),
        transport: support::transport(),
        template_manager: OnceCell::const_new(),
    };

    let error = manager
        .get_prompt_template("absent", None)
        .await
        .expect_err("the template is not cached");

    assert_eq!(error.to_string(), "Prompt template 'absent' not found");
    assert_eq!(
        server.request_count(),
        1,
        "a missing lookup never reaches BitBucket"
    );
}
