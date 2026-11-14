use async_trait::async_trait;
use betterlite_models::decisions::Json;

pub mod genai;
pub mod responses;
pub mod tokenizer;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranslatedChat {
    pub messages: Json,
    pub tools: Option<Json>,
    pub tool_choice: Option<Json>,
}

pub struct GenAiTranslationRequest<'request> {
    pub model: &'request str,
    pub contents: &'request Json,
    pub config: Option<&'request Json>,
    pub system_instruction: Option<&'request Json>,
    pub system_instruction_snake: Option<&'request Json>,
    pub tools: Option<&'request Json>,
    pub tool_config: Option<&'request Json>,
    pub tool_config_snake: Option<&'request Json>,
}

pub trait GenAiTranslation: Send + Sync {
    fn translate_generate_content(
        &self,
        request: &GenAiTranslationRequest<'_>,
    ) -> Option<TranslatedChat>;
}

pub struct ResponsesInputRequest<'request> {
    pub input: &'request Json,
    pub instructions: Option<&'request str>,
}

pub trait ResponsesInput: Send + Sync {
    fn transform_responses_api_input(&self, request: &ResponsesInputRequest<'_>) -> Vec<Json>;
}

pub struct TokenCounterRequest<'request> {
    pub model: &'request str,
    pub messages: Option<&'request [Json]>,
    pub text: Option<&'request Json>,
    pub tools: Option<&'request [Json]>,
    pub tool_choice: Option<&'request Json>,
}

#[async_trait]
pub trait InputTokenCounter: Send + Sync {
    // None is LiteLLM's bare except: any counting failure degrades to the char-count estimate
    async fn count_tokens(&self, request: &TokenCounterRequest<'_>) -> Option<i64>;
}
