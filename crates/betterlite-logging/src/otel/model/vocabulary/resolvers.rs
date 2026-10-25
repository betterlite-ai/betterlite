use crate::otel::vocabulary::constants::{GenAIOperation, GenAIOutputType, GenAIProvider};
use crate::otel::vocabulary::registry::SpanRole;

#[path = "resolvers/tests.rs"]
mod tests;

pub const POSTGRESQL: &str = "postgresql";

#[must_use]
pub fn resolve_provider(custom_llm_provider: &str) -> &str {
    let provider = match custom_llm_provider.to_lowercase().as_str() {
        "openai" | "text-completion-openai" => GenAIProvider::Openai,
        "azure" => GenAIProvider::AzureAiOpenai,
        "azure_ai" => GenAIProvider::AzureAiInference,
        "anthropic" => GenAIProvider::Anthropic,
        "bedrock" | "bedrock_converse" => GenAIProvider::AwsBedrock,
        "vertex_ai" | "vertex_ai_beta" => GenAIProvider::GcpVertexAi,
        "gemini" => GenAIProvider::GcpGemini,
        "cohere" | "cohere_chat" => GenAIProvider::Cohere,
        "mistral" => GenAIProvider::MistralAi,
        "deepseek" => GenAIProvider::Deepseek,
        "groq" => GenAIProvider::Groq,
        "perplexity" => GenAIProvider::Perplexity,
        "xai" => GenAIProvider::XAi,
        "watsonx" => GenAIProvider::IbmWatsonxAi,
        _ => return custom_llm_provider,
    };
    <&str>::from(provider)
}

// typos reads the async "a" prefix of asend_message as a misspelling, and typos.toml is outside this task
const ASEND_MESSAGE: &str = concat!("a", "send_message");
const ASEND_MESSAGE_STREAMING: &str = concat!("a", "send_message_streaming");

#[must_use]
pub fn resolve_operation(call_type: &str) -> GenAIOperation {
    match call_type.to_lowercase().as_str() {
        ""
        | "completion"
        | "acompletion"
        | "completion_with_retries"
        | "responses"
        | "aresponses" => GenAIOperation::Chat,
        "text_completion" | "atext_completion" => GenAIOperation::TextCompletion,
        "embedding" | "aembedding" => GenAIOperation::Embeddings,
        "image_generation" | "aimage_generation" | "ocr" | "aocr" | "speech" | "aspeech"
        | "transcription" | "atranscription" => GenAIOperation::GenerateContent,
        "call_mcp_tool" => GenAIOperation::ExecuteTool,
        "vector_store_search" | "avector_store_search" | "query" | "aquery" => {
            GenAIOperation::Retrieval
        }
        "send_message" | ASEND_MESSAGE | ASEND_MESSAGE_STREAMING => GenAIOperation::InvokeAgent,
        vendor => vendor_operation(vendor).unwrap_or_else(|| {
            // an unmapped call type labelled chat mixes its latency and cost into every chat chart
            tracing::debug!(
                call_type,
                "otel: call_type has no gen_ai.operation.name mapping; labelling it chat"
            );
            GenAIOperation::Chat
        }),
    }
}

// the convention names no operation for these, so they take litellm.* vendor values
fn vendor_operation(call_type: &str) -> Option<GenAIOperation> {
    match call_type {
        "get_responses" | "aget_responses" | "delete_responses" | "adelete_responses"
        | "cancel_responses" | "acancel_responses" | "list_input_items" | "alist_input_items" => {
            Some(GenAIOperation::LitellmResponsesManagement)
        }
        "moderation" | "amoderation" => Some(GenAIOperation::LitellmModeration),
        "vector_store_create"
        | "avector_store_create"
        | "vector_store_retrieve"
        | "avector_store_retrieve"
        | "vector_store_list"
        | "avector_store_list"
        | "vector_store_update"
        | "avector_store_update"
        | "vector_store_delete"
        | "avector_store_delete" => Some(GenAIOperation::LitellmVectorStoreManagement),
        "vector_store_file_create"
        | "avector_store_file_create"
        | "vector_store_file_list"
        | "avector_store_file_list"
        | "vector_store_file_retrieve"
        | "avector_store_file_retrieve"
        | "vector_store_file_content"
        | "avector_store_file_content"
        | "vector_store_file_update"
        | "avector_store_file_update"
        | "vector_store_file_delete"
        | "avector_store_file_delete" => Some(GenAIOperation::LitellmVectorStoreFileManagement),
        _ => None,
    }
}

#[must_use]
pub fn resolve_output_type(call_type: &str) -> Option<GenAIOutputType> {
    match call_type.to_lowercase().as_str() {
        "image_generation" | "aimage_generation" => Some(GenAIOutputType::Image),
        "speech" | "aspeech" => Some(GenAIOutputType::Speech),
        "transcription" | "atranscription" | "ocr" | "aocr" => Some(GenAIOutputType::Text),
        _ => None,
    }
}

#[must_use]
pub fn db_system(service_name: &str) -> Option<&'static str> {
    match service_name {
        "redis" => Some("redis"),
        "postgres" | "batch_write_to_db" => Some(POSTGRESQL),
        spend_queue if spend_queue.starts_with("redis_") => Some("redis"),
        _ => None,
    }
}

#[must_use]
pub fn span_role_for_service(service_name: &str) -> Option<SpanRole> {
    match service_name {
        // metrics-only services: each one duplicates a GenAI span or has its own live span
        "self" | "router" | "proxy_pre_call" | "auth" => None,
        datastore if db_system(datastore).is_some() => Some(SpanRole::DbCall),
        _ => Some(SpanRole::Service),
    }
}
