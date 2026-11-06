#![cfg(test)]

use rstest::rstest;

use super::id_to_repo_path;

#[rstest]
#[case::scoped("prompts", "chat/greeting", "prompts/chat/greeting.prompt")]
#[case::unscoped("", "chat/greeting", "chat/greeting.prompt")]
#[case::encoded_id("prompts", "gitlab::chat::greeting", "prompts/chat/greeting.prompt")]
#[case::nested_scope("prompts/chat", "hi", "prompts/chat/hi.prompt")]
#[case::encoded_id_unscoped("", "gitlab::hi", "hi.prompt")]
fn an_id_maps_to_a_prompt_file_under_the_scope(
    #[case] prompts_path: &str,
    #[case] prompt_id: &str,
    #[case] expected: &str,
) {
    assert_eq!(id_to_repo_path(prompts_path, prompt_id), expected);
}
