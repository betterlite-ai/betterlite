use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;

#[test]
fn should_run_prompt_management_is_true_only_with_a_prompt_id() {
    assert!(BitBucketPromptManager::should_run_prompt_management(Some(
        "greeting"
    )));
    assert!(
        BitBucketPromptManager::should_run_prompt_management(Some("")),
        "LiteLLM compares the id against None, so an empty id still runs"
    );
    assert!(!BitBucketPromptManager::should_run_prompt_management(None));
}
