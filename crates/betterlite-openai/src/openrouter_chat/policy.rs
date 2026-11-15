use betterlite_prompts::arguments::json::Object;
use serde_json::Value;

use crate::chat::ChatMessage;
use crate::openai_base_config::Headers;
use crate::openrouter_chat::error::OpenRouterError;

// B503 builds this body after OpenrouterConfig decided with remove_cache_control whether a model keeps its flags
pub struct BaseRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub litellm_params: &'request Object,
    pub headers: &'request Headers,
}

// B668, B503 and B504 supply these calls at composition; this module never imports their internals
pub trait ChatPolicy {
    fn supported_params(&self, model: &str) -> Vec<&'static str>;

    fn map_params(&self, model: &str, requested: &Object, optional_params: &mut Object);

    fn remove_cache_control(&self, messages: &mut Vec<ChatMessage>, optional_params: &mut Object);

    fn request_body(&self, request: BaseRequest<'_>) -> Result<Object, OpenRouterError>;

    fn response_body(&self, raw: &[u8]) -> Result<Value, OpenRouterError>;
}
