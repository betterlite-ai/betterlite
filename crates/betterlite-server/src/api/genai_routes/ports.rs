use std::sync::Arc;

use async_trait::async_trait;
use axum::http::HeaderMap;
use betterlite_core::api::genai_chat_output::GenAiOutputError;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_openai::chat::ChatMessage;
use betterlite_prompts::arguments::json::{Json, Object};
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};

use crate::auth::auth_identity::AuthContext;

// what api::genai_routes::routes needs of the proxy: the request pipeline, the adapter and the internal counter
#[derive(Clone)]
pub struct GenaiRoutesState {
    pub generation: Arc<dyn GenerateContentDispatch>,
    pub translator: Arc<dyn TranslateContents>,
    pub token_counter: Arc<dyn TokenCounter>,
    // general_settings.include_call_id_in_error_body: openai_exception_handler copies x-litellm-call-id into the error body
    pub include_call_id_in_error_body: bool,
}

// one call of Processor.base_process_llm_request: the body after the model and stream fields are set, the path model, the caller and the request headers
pub struct GenerateContentCall<'call> {
    pub model: &'call str,
    pub data: &'call Object,
    pub context: &'call AuthContext,
    pub headers: &'call HeaderMap,
}

// what the pipeline answers: a JSON document or the provider's raw SSE chunks
pub enum GenerationBody {
    Json(Json),
    Events(BoxStream<'static, Result<Vec<u8>, GenAiOutputError>>),
}

pub struct GenerationAnswer {
    pub headers: HeaderMap,
    pub body: GenerationBody,
}

// base_process_llm_request under route_type agenerate_content and agenerate_content_stream
#[async_trait]
pub trait GenerateContentDispatch: Send + Sync {
    async fn generate(
        &self,
        call: GenerateContentCall<'_>,
    ) -> Result<GenerationAnswer, Box<ProxyException>>;

    async fn stream(
        &self,
        call: GenerateContentCall<'_>,
    ) -> Result<GenerationAnswer, Box<ProxyException>>;
}

// GoogleGenAIAdapter.translate_generate_content_to_completion().get("messages", [])
pub trait TranslateContents: Send + Sync {
    fn messages(
        &self,
        model: &str,
        contents: &Json,
    ) -> Result<Vec<ChatMessage>, Box<ProxyException>>;
}

// proxy_server.token_counter(request=TokenCountRequest(..), call_endpoint=True)
#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_tokens(
        &self,
        request: &TokenCountRequest,
        call_endpoint: bool,
    ) -> Result<Option<TokenCountResponse>, Box<ProxyException>>;
}

// LiteLLM's TokenCountRequest cut down to the fields this route builds
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenCountRequest {
    pub model: String,
    pub contents: Json,
    pub messages: Vec<ChatMessage>,
}

// LiteLLM's TokenCountResponse cut down to the fields this route reads
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenCountResponse {
    pub total_tokens: i64,
    pub original_response: Option<TokenCountOriginal>,
}

// TokenCountDetailsResponse's source: the provider's own count reply
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenCountOriginal {
    pub total_tokens: Option<i64>,
    pub prompt_tokens_details: Option<Vec<PromptTokenDetail>>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTokenDetail {
    pub modality: Modality,
    pub token_count: i64,
}

// PromptTokensDetails.modality's four literals
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Modality {
    Text,
    Audio,
    Image,
    Video,
}
