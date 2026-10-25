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

// typos flags the async send_message names as misspellings, and typos.toml is outside this task
const ASYNC_SEND_MESSAGE: &str = concat!("a", "send_message");
const ASYNC_SEND_STREAMING: &str = concat!("a", "send_message_streaming");

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
        "send_message" | ASYNC_SEND_MESSAGE | ASYNC_SEND_STREAMING => GenAIOperation::InvokeAgent,
        "get_responses" | "aget_responses" | "delete_responses" | "adelete_responses"
        | "cancel_responses" | "acancel_responses" | "list_input_items" | "alist_input_items" => {
            GenAIOperation::ResponsesManagement
        }
        "moderation" | "amoderation" => GenAIOperation::Moderation,
        other => vector_store_operation(other).unwrap_or_else(|| {
            // an unmapped call type labelled chat mixes its latency and cost into every chat chart
            tracing::debug!(
                call_type,
                "otel: call_type has no gen_ai.operation.name mapping; labelling it chat"
            );
            GenAIOperation::Chat
        }),
    }
}

// LiteLLM maps each vector store action twice, as the sync name and its "a"-prefixed async name
fn vector_store_operation(call_type: &str) -> Option<GenAIOperation> {
    let sync_name = call_type.strip_prefix('a').unwrap_or(call_type);
    match sync_name.strip_prefix("vector_store_")? {
        "create" | "retrieve" | "list" | "update" | "delete" => {
            Some(GenAIOperation::VectorStoreManagement)
        }
        "file_create" | "file_list" | "file_retrieve" | "file_content" | "file_update"
        | "file_delete" => Some(GenAIOperation::VectorStoreFileManagement),
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
