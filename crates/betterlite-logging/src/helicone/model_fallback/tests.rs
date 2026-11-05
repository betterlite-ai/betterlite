#![cfg(test)]

use rstest::rstest;

use super::resolve;

#[rstest]
#[case::gpt_kept("gpt-4o", "openai", "gpt-4o", false, false)]
#[case::unknown_falls_back("mistral-large", "mistral", "gpt-3.5-turbo", false, false)]
#[case::claude_kept_in_claude_format("claude-3-opus", "anthropic", "claude-3-opus", false, true)]
#[case::gemini_kept("gemini-pro", "gemini", "gemini-pro", false, false)]
#[case::command_r_kept("command-r-plus", "cohere", "command-r-plus", false, false)]
#[case::command_nightly_kept("command-nightly", "cohere", "command-nightly", false, false)]
#[case::substring_is_enough("my-gpt-finetune", "openai", "my-gpt-finetune", false, false)]
#[case::unknown_command_falls_back("command-a", "cohere", "gpt-3.5-turbo", false, false)]
#[case::vertex_provider_keeps_any_name("llama-3", "vertex_ai", "llama-3", true, false)]
#[case::vertex_prefix_keeps_any_name("vertex_ai/llama-3", "", "vertex_ai/llama-3", true, false)]
#[case::vertex_claude_is_not_claude_format(
    "vertex_ai/claude-3",
    "",
    "vertex_ai/claude-3",
    true,
    false
)]
#[case::vertex_provider_with_claude_name("claude-3", "vertex_ai", "claude-3", true, false)]
#[case::vertex_must_be_a_prefix("my-vertex_ai/llama", "", "gpt-3.5-turbo", false, false)]
fn model_fallback_selects_the_name(
    #[case] model: &str,
    #[case] provider: &str,
    #[case] name: &str,
    #[case] vertex_ai: bool,
    #[case] claude_format: bool,
) {
    let resolved = resolve(model, provider);
    assert_eq!(resolved.name, name);
    assert_eq!(resolved.vertex_ai, vertex_ai);
    assert_eq!(resolved.claude_format, claude_format);
}
