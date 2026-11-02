#![cfg(test)]

use rstest::rstest;

use super::classify_llm_route;

#[rstest]
#[case::exact_v1_messages("/v1/messages", Some("/v1/messages"))]
#[case::exact_interactions("/interactions", Some("/interactions"))]
#[case::exact_v1beta_interactions("/v1beta/interactions", Some("/v1beta/interactions"))]
#[case::exact_comprehendmedical("/comprehendmedical", Some("/comprehendmedical"))]
#[case::exact_transcribe("/transcribe", Some("/transcribe"))]
#[case::interaction_id_read_excluded("/interactions/abc123", None)]
#[case::interaction_cancel_excluded("/interactions/abc123/cancel", None)]
#[case::chat_completions_suffix_not_bare_completions(
    "/v1/chat/completions",
    Some("/chat/completions")
)]
#[case::bare_completions_suffix("/v1/completions", Some("/completions"))]
#[case::embeddings_suffix("/v1/embeddings", Some("/embeddings"))]
#[case::gemini_generate_content(
    "/v1beta/models/gemini-pro:generateContent",
    Some(":generateContent")
)]
#[case::gemini_stream_generate_content(
    "/v1beta/models/gemini-pro:streamGenerateContent",
    Some(":streamGenerateContent")
)]
#[case::assistants_messages_not_billable("/v1/threads/thread_1/messages", None)]
#[case::bedrock_passthrough_deep("/bedrock/anthropic.claude-3-sonnet", Some("/bedrock"))]
#[case::bedrock_passthrough_bare_prefix_excluded("/bedrock", None)]
#[case::langfuse_excluded("/langfuse/api/public/ingestion", None)]
#[case::vertex_ai_dash_passthrough("/vertex-ai/v1/projects/x", Some("/vertex-ai"))]
#[case::vertex_ai_underscore_passthrough("/vertex_ai/v1/projects/x", Some("/vertex_ai"))]
#[case::unrelated_path("/health/liveliness", None)]
fn classifies_llm_routes(#[case] path: &str, #[case] expected: Option<&str>) {
    assert_eq!(classify_llm_route(path), expected);
}
