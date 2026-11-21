use super::error::CompletionCallError;
use super::tokens::TokenCountResponse;
use crate::call_setup::correlation::CallOutcome;
use crate::completion_request::call::PreparedCall;
use async_trait::async_trait;
use betterlite_core::responses_bridge::ReasoningEffort;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use secrecy::SecretString;
use std::time::Duration;
#[async_trait]
pub trait Sleeper: Sync {
    async fn sleep(&self, duration: Duration);
}
#[async_trait]
pub trait McpLoopRunner: Sync {
    async fn run(
        &self,
        call: &PreparedCall,
        acompletion: bool,
    ) -> Result<CallOutcome, CompletionCallError>;
}
pub struct NoMcpLoop;
#[async_trait]
impl McpLoopRunner for NoMcpLoop {
    async fn run(
        &self,
        _call: &PreparedCall,
        _acompletion: bool,
    ) -> Result<CallOutcome, CompletionCallError> {
        Err(CompletionCallError::NoMcpLoop)
    }
}
pub trait FoundryRules: Sync {
    fn supports_responses(&self, model: &str, api_base: Option<&str>) -> bool;
    fn chat_rejects_tools(&self, model: &str, reasoning_effort: Option<&ReasoningEffort>) -> bool;
}
pub struct NoFoundryRules;
impl FoundryRules for NoFoundryRules {
    fn supports_responses(&self, _model: &str, _api_base: Option<&str>) -> bool {
        false
    }
    fn chat_rejects_tools(&self, _model: &str, _effort: Option<&ReasoningEffort>) -> bool {
        false
    }
}
pub trait ModelFacts: Sync {
    fn model_info(&self, model: &str, custom_llm_provider: &str) -> Option<Object>;
}
pub struct ProviderTokenCountRequest<'request> {
    pub model_to_use: &'request str,
    pub request_model: &'request str,
    pub messages: Option<&'request [Json]>,
    pub tools: Option<&'request [Json]>,
    pub system: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
}
#[async_trait]
pub trait ProviderTokenCounter: Sync {
    fn should_use(&self, custom_llm_provider: &str) -> bool;
    async fn count(&self, request: &ProviderTokenCountRequest<'_>) -> Option<TokenCountResponse>;
}
pub struct LocalTokenCountRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub tools: Option<&'request [Json]>,
}
#[async_trait]
pub trait LocalTokenCounter: Sync {
    async fn count(&self, request: &LocalTokenCountRequest<'_>) -> usize;
}
