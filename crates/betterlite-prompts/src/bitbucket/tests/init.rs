use tokio::sync::OnceCell;

use betterlite_prompts::bitbucket::initializer::prompt_initializer;
use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;

use crate::support;

#[tokio::test]
async fn init_accepts_the_config_without_building_the_template_manager() {
    let manager = BitBucketPromptManager {
        bitbucket_config: support::config(support::CLOSED_BASE),
        prompt_id: Some("greeting".to_owned()),
        transport: support::transport(),
        template_manager: OnceCell::const_new(),
    };

    let error = manager
        .prompt_manager()
        .await
        .expect_err("the first access builds the manager and fails on the closed port");
    assert!(error.to_string().contains("greeting"));
}

#[test]
fn integration_name_is_bitbucket() {
    assert_eq!(BitBucketPromptManager::INTEGRATION_NAME, "bitbucket");
}

#[tokio::test]
async fn initializer_builds_a_manager_from_litellm_params() {
    let server = support::Server::start(&[("greeting.prompt", 200, "Hello")]);
    let params = support::params(Some("greeting"), &server.base_url);
    let spec = support::prompt_spec("registered");

    let manager = prompt_initializer(&params, &spec, support::transport()).expect("initializer");
    let (rendered, _) = manager
        .get_prompt_template("greeting", None)
        .await
        .expect("the configured prompt loads through the config the initializer read");

    assert_eq!(rendered, "Hello");
    assert_eq!(server.request_count(), 1);
}

#[test]
fn initializer_requires_bitbucket_config() {
    let params = support::params_without_config();
    let spec = support::prompt_spec("registered");

    let error = prompt_initializer(&params, &spec, support::transport())
        .expect_err("bitbucket_config is required");

    assert_eq!(
        error.to_string(),
        "bitbucket_config is required for BitBucket prompt integration"
    );
}
