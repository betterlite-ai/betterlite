use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;
use tokio::sync::OnceCell;

use crate::support;

#[tokio::test]
async fn should_run_prompt_management_runs_an_empty_id_but_not_a_missing_one() {
    let server = support::Server::start(&[]);
    let manager = BitBucketPromptManager {
        bitbucket_config: support::config(&server.base_url),
        prompt_id: None,
        transport: support::transport(),
        template_manager: OnceCell::const_new(),
    };

    let missing = manager
        .compile_prompt_helper(None, None, None, None)
        .await
        .expect_err("a missing id does not run the manager");
    assert_eq!(
        missing.to_string(),
        "prompt_id is required for BitBucket prompt manager"
    );

    let empty = manager
        .compile_prompt_helper(Some(""), None, None, None)
        .await
        .expect_err("an empty id still runs the manager, and its prompt is missing");
    assert_eq!(
        empty.to_string(),
        "Error compiling prompt '': Prompt template '' not found"
    );
}
