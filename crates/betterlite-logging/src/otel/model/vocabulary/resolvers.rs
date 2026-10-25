use super::constants::{GenAIOperation, GenAIOutputType, GenAIProvider};
use super::registry::SpanRole;

const CHAT_CALL_TYPES: &[&str] = &[
    "completion",
    "acompletion",
    "completion_with_retries",
    "responses",
    "aresponses",
];
const RESPONSE_CALL_TYPES: &[&str] = &[
    "get_responses",
    "aget_responses",
    "delete_responses",
    "adelete_responses",
    "cancel_responses",
    "acancel_responses",
    "list_input_items",
    "alist_input_items",
];
const CONTENT_CALL_TYPES: &[&str] = &[
    "image_generation",
    "aimage_generation",
    "ocr",
    "aocr",
    "speech",
    "aspeech",
    "transcription",
    "atranscription",
];
const RETRIEVAL_CALL_TYPES: &[&str] = &[
    "vector_store_search",
    "avector_store_search",
    "query",
    "aquery",
];
const AGENT_CALL_TYPES: &[&str] = &["send_message", "asend_message", "asend_message_streaming"];
const VECTOR_CALL_TYPES: &[&str] = &[
    "vector_store_create",
    "avector_store_create",
    "vector_store_retrieve",
    "avector_store_retrieve",
    "vector_store_list",
    "avector_store_list",
    "vector_store_update",
    "avector_store_update",
    "vector_store_delete",
    "avector_store_delete",
];
const VECTOR_FILE_CALL_TYPES: &[&str] = &[
    "vector_store_file_create",
    "avector_store_file_create",
    "vector_store_file_list",
    "avector_store_file_list",
    "vector_store_file_retrieve",
    "avector_store_file_retrieve",
    "vector_store_file_content",
    "avector_store_file_content",
    "vector_store_file_update",
    "avector_store_file_update",
    "vector_store_file_delete",
    "avector_store_file_delete",
];

#[must_use]
pub fn resolve_provider(provider: Option<&str>) -> String {
    let Some(value) = provider else {
        return String::new();
    };
    let mapped = match value.to_ascii_lowercase().as_str() {
        "openai" | "text-completion-openai" => Some(GenAIProvider::OPENAI),
        "azure" => Some(GenAIProvider::AZURE_AI_OPENAI),
        "azure_ai" => Some(GenAIProvider::AZURE_AI_INFERENCE),
        "anthropic" => Some(GenAIProvider::ANTHROPIC),
        "bedrock" | "bedrock_converse" => Some(GenAIProvider::AWS_BEDROCK),
        "vertex_ai" | "vertex_ai_beta" => Some(GenAIProvider::GCP_VERTEX_AI),
        "gemini" => Some(GenAIProvider::GCP_GEMINI),
        "cohere" | "cohere_chat" => Some(GenAIProvider::COHERE),
        "mistral" => Some(GenAIProvider::MISTRAL_AI),
        "deepseek" => Some(GenAIProvider::DEEPSEEK),
        "groq" => Some(GenAIProvider::GROQ),
        "perplexity" => Some(GenAIProvider::PERPLEXITY),
        "xai" => Some(GenAIProvider::X_AI),
        "watsonx" => Some(GenAIProvider::IBM_WATSONX_AI),
        _ => None,
    };
    mapped.map_or_else(
        || value.to_owned(),
        |mapped_provider| mapped_provider.value().to_owned(),
    )
}

#[must_use]
pub fn resolve_operation(call_type: Option<&str>) -> GenAIOperation {
    let Some(value) = call_type.filter(|value| !value.is_empty()) else {
        return GenAIOperation::CHAT;
    };
    let normalized = value.to_ascii_lowercase();
    if CHAT_CALL_TYPES.contains(&normalized.as_str()) {
        return GenAIOperation::CHAT;
    }
    if normalized == "text_completion" || normalized == "atext_completion" {
        return GenAIOperation::TEXT_COMPLETION;
    }
    if normalized == "embedding" || normalized == "aembedding" {
        return GenAIOperation::EMBEDDINGS;
    }
    if RESPONSE_CALL_TYPES.contains(&normalized.as_str()) {
        return GenAIOperation::LITELLM_RESPONSES_MANAGEMENT;
    }
    if CONTENT_CALL_TYPES.contains(&normalized.as_str()) {
        return GenAIOperation::GENERATE_CONTENT;
    }
    if normalized == "moderation" || normalized == "amoderation" {
        return GenAIOperation::LITELLM_MODERATION;
    }
    if normalized == "call_mcp_tool" {
        return GenAIOperation::EXECUTE_TOOL;
    }
    if RETRIEVAL_CALL_TYPES.contains(&normalized.as_str()) {
        return GenAIOperation::RETRIEVAL;
    }
    if AGENT_CALL_TYPES.contains(&normalized.as_str()) {
        return GenAIOperation::INVOKE_AGENT;
    }
    if VECTOR_CALL_TYPES.contains(&normalized.as_str()) {
        return GenAIOperation::LITELLM_VECTOR_STORE_MANAGEMENT;
    }
    if VECTOR_FILE_CALL_TYPES.contains(&normalized.as_str()) {
        return GenAIOperation::LITELLM_VECTOR_STORE_FILE_MANAGEMENT;
    }
    tracing::debug!(call_type = value, "unmapped OTel call type; using chat");
    GenAIOperation::CHAT
}

#[must_use]
pub fn resolve_output_type(call_type: Option<&str>) -> Option<GenAIOutputType> {
    let value = call_type?;
    match value.to_ascii_lowercase().as_str() {
        "image_generation" | "aimage_generation" => Some(GenAIOutputType::IMAGE),
        "speech" | "aspeech" => Some(GenAIOutputType::SPEECH),
        "transcription" | "atranscription" | "ocr" | "aocr" => Some(GenAIOutputType::TEXT),
        _ => None,
    }
}

#[must_use]
pub fn db_system(service_name: &str) -> Option<&'static str> {
    match service_name {
        "redis" => Some("redis"),
        "postgres" | "batch_write_to_db" => Some("postgresql"),
        value if value.starts_with("redis_") => Some("redis"),
        _ => None,
    }
}

#[must_use]
pub fn span_role_for_service(service_name: &str) -> Option<SpanRole> {
    match service_name {
        "self" | "router" | "proxy_pre_call" | "auth" => None,
        value if db_system(value).is_some() => Some(SpanRole::DB_CALL),
        _ => Some(SpanRole::SERVICE),
    }
}

#[cfg(test)]
#[path = "resolvers/tests.rs"]
mod tests;
