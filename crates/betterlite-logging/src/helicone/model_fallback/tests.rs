#![cfg(test)]

use rstest::rstest;

use super::{Provider, resolve};

#[rstest]
#[case::gpt_kept("gpt-4o", "openai", "gpt-4o", Provider::OpenAi)]
#[case::unknown_falls_back("mistral-large", "mistral", "gpt-3.5-turbo", Provider::OpenAi)]
#[case::claude_kept("claude-3-opus", "anthropic", "claude-3-opus", Provider::Claude)]
#[case::gemini_kept("gemini-pro", "gemini", "gemini-pro", Provider::Gemini)]
#[case::command_r_kept("command-r-plus", "cohere", "command-r-plus", Provider::OpenAi)]
#[case::command_nightly_kept("command-nightly", "cohere", "command-nightly", Provider::OpenAi)]
#[case::substring_is_enough("my-gpt-finetune", "openai", "my-gpt-finetune", Provider::OpenAi)]
#[case::unknown_command_falls_back("command-a", "cohere", "gpt-3.5-turbo", Provider::OpenAi)]
#[case::claude_beats_gemini("claude-gemini-x", "", "claude-gemini-x", Provider::Claude)]
#[case::vertex_provider_keeps_any_name("llama-3", "vertex_ai", "llama-3", Provider::VertexAi)]
#[case::vertex_prefix_keeps_any_name(
    "vertex_ai/llama-3",
    "",
    "vertex_ai/llama-3",
    Provider::VertexAi
)]
#[case::vertex_claude_stays_vertex(
    "vertex_ai/claude-3",
    "",
    "vertex_ai/claude-3",
    Provider::VertexAi
)]
#[case::vertex_provider_with_claude_name("claude-3", "vertex_ai", "claude-3", Provider::VertexAi)]
#[case::vertex_provider_with_gemini_name(
    "gemini-pro",
    "vertex_ai",
    "gemini-pro",
    Provider::VertexAi
)]
#[case::vertex_must_be_a_prefix("my-vertex_ai/llama", "", "gpt-3.5-turbo", Provider::OpenAi)]
fn model_fallback_selects_the_name(
    #[case] model: &str,
    #[case] custom_llm_provider: &str,
    #[case] name: &str,
    #[case] provider: Provider,
) {
    let resolved = resolve(model, custom_llm_provider);
    assert_eq!(resolved.name, name);
    assert_eq!(resolved.provider, provider);
}
