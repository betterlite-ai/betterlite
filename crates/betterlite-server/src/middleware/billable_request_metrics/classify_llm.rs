mod tests;

const LLM_ROUTE_EXACT: &[&str] = &[
    "/v1/messages",
    "/interactions",
    "/v1beta/interactions",
    "/comprehendmedical",
    "/transcribe",
];

// Ordered: a longer suffix sharing an ending with a shorter one comes first, e.g. "/chat/completions" before "/completions".
const LLM_ROUTE_SUFFIXES: &[&str] = &[
    "/chat/completions",
    "/completions",
    "/embeddings",
    "/responses",
    "/rerank",
    "/moderations",
    "/images/generations",
    "/images/edits",
    "/images/variations",
    "/audio/transcriptions",
    "/audio/translations",
    "/audio/speech",
    "/videos",
    "/remix",
    "/ocr",
    "/search",
    "/rag/query",
    "/rag/ingest",
    ":generateContent",
    ":streamGenerateContent",
];

// LiteLLMRoutes.mapped_pass_through_routes, excluding /langfuse since it forwards observability traffic, not inference.
const PASSTHROUGH_PREFIXES: &[&str] = &[
    "/bedrock",
    "/comprehendmedical",
    "/azure_speech",
    "/transcribe",
    "/vertex-ai",
    "/vertex_ai",
    "/cohere",
    "/cursor",
    "/gemini",
    "/anthropic",
    "/azure",
    "/azure_ai",
    "/openai",
    "/openai_passthrough",
    "/assemblyai",
    "/eu.assemblyai",
    "/tinyfish",
    "/vllm",
    "/mistral",
    "/typesafe",
    "/openrouter",
    "/milvus",
    "/gigachat",
    "/watsonx",
    "/nvidia_nim",
    "/deepgram",
    "/fal_ai",
];

pub(super) fn classify_llm_route(path: &str) -> Option<&'static str> {
    if let Some(exact) = LLM_ROUTE_EXACT.iter().copied().find(|&route| path == route) {
        return Some(exact);
    }
    if let Some(suffix) = LLM_ROUTE_SUFFIXES
        .iter()
        .copied()
        .find(|&suffix| path == suffix || path.ends_with(suffix))
    {
        return Some(suffix);
    }
    // Deep passthrough paths only: the bare prefix itself is not an inference call.
    PASSTHROUGH_PREFIXES.iter().copied().find(|&prefix| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
    })
}
