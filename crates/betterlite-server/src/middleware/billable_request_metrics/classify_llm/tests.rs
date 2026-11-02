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
#[case::responses_suffix("/v1/responses", Some("/responses"))]
#[case::rerank_suffix("/v1/rerank", Some("/rerank"))]
#[case::moderations_suffix("/v1/moderations", Some("/moderations"))]
#[case::images_generations_suffix("/v1/images/generations", Some("/images/generations"))]
#[case::images_edits_suffix("/v1/images/edits", Some("/images/edits"))]
#[case::images_variations_suffix("/v1/images/variations", Some("/images/variations"))]
#[case::audio_transcriptions_suffix("/v1/audio/transcriptions", Some("/audio/transcriptions"))]
#[case::audio_translations_suffix("/v1/audio/translations", Some("/audio/translations"))]
#[case::audio_speech_suffix("/v1/audio/speech", Some("/audio/speech"))]
#[case::videos_suffix("/v1/videos", Some("/videos"))]
#[case::videos_remix_suffix("/v1/videos/vid_1/remix", Some("/remix"))]
#[case::ocr_suffix("/v1/ocr", Some("/ocr"))]
#[case::vector_stores_search_suffix("/v1/vector_stores/vs_1/search", Some("/search"))]
#[case::rag_query_suffix("/v1/rag/query", Some("/rag/query"))]
#[case::rag_ingest_suffix("/v1/rag/ingest", Some("/rag/ingest"))]
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
#[case::comprehendmedical_passthrough_deep(
    "/comprehendmedical/some-operation",
    Some("/comprehendmedical")
)]
#[case::azure_speech_passthrough_deep("/azure_speech/v1/token", Some("/azure_speech"))]
#[case::transcribe_passthrough_deep("/transcribe/some-job", Some("/transcribe"))]
#[case::langfuse_excluded("/langfuse/api/public/ingestion", None)]
#[case::vertex_ai_dash_passthrough("/vertex-ai/v1/projects/x", Some("/vertex-ai"))]
#[case::vertex_ai_underscore_passthrough("/vertex_ai/v1/projects/x", Some("/vertex_ai"))]
#[case::cohere_passthrough_deep("/cohere/v1/generate", Some("/cohere"))]
#[case::cursor_passthrough_deep("/cursor/v1/chat", Some("/cursor"))]
#[case::gemini_passthrough_deep("/gemini/v1beta/models/gemini-pro", Some("/gemini"))]
#[case::anthropic_passthrough_deep("/anthropic/v1/messages", Some("/anthropic"))]
#[case::azure_passthrough_deep("/azure/some/resource", Some("/azure"))]
#[case::azure_ai_passthrough_deep("/azure_ai/v1/chat", Some("/azure_ai"))]
#[case::openai_passthrough_deep("/openai/v1/models", Some("/openai"))]
#[case::openai_passthrough_passthrough_deep(
    "/openai_passthrough/v1/files",
    Some("/openai_passthrough")
)]
#[case::assemblyai_passthrough_deep("/assemblyai/v2/transcript", Some("/assemblyai"))]
#[case::eu_assemblyai_passthrough_deep("/eu.assemblyai/v2/transcript", Some("/eu.assemblyai"))]
#[case::tinyfish_passthrough_deep("/tinyfish/v1/run", Some("/tinyfish"))]
#[case::vllm_passthrough_deep("/vllm/v1/models", Some("/vllm"))]
#[case::mistral_passthrough_deep("/mistral/v1/chat", Some("/mistral"))]
#[case::typesafe_passthrough_deep("/typesafe/v1/run", Some("/typesafe"))]
#[case::openrouter_passthrough_deep("/openrouter/v1/models", Some("/openrouter"))]
#[case::milvus_passthrough_deep("/milvus/v1/collections", Some("/milvus"))]
#[case::gigachat_passthrough_deep("/gigachat/v1/models", Some("/gigachat"))]
#[case::watsonx_passthrough_deep("/watsonx/v1/deployments", Some("/watsonx"))]
#[case::nvidia_nim_passthrough_deep("/nvidia_nim/v1/models", Some("/nvidia_nim"))]
#[case::deepgram_passthrough_deep("/deepgram/v1/listen", Some("/deepgram"))]
#[case::fal_ai_passthrough_deep("/fal_ai/v1/run", Some("/fal_ai"))]
#[case::unrelated_path("/health/liveliness", None)]
fn classifies_llm_routes(#[case] path: &str, #[case] expected: Option<&str>) {
    assert_eq!(classify_llm_route(path), expected);
}
