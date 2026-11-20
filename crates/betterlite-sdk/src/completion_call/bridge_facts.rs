use super::ports::{FoundryRules, ModelFacts};
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_core::responses_bridge::{
    ModelInfoError, ReasoningEffort, ResponsesBridgeCapabilities,
};
use betterlite_openai::credentials::resolve_openai_api_base;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::reasoning_models::{
    is_model_gpt_5_4_plus_model, is_model_gpt_5_search_model,
};
pub struct ModelBridgeFacts<'facts, Rules: FoundryRules + ?Sized> {
    pub facts: &'facts dyn ModelFacts,
    pub foundry: &'facts Rules,
    pub openai: OpenAiCredentialsSettings,
    pub litellm_api_base: Option<String>,
}
impl<Rules: FoundryRules + ?Sized> ResponsesBridgeCapabilities for ModelBridgeFacts<'_, Rules> {
    fn model_info(&self, model: &str, custom_llm_provider: &str) -> Result<Object, ModelInfoError> {
        self.facts
            .model_info(model, custom_llm_provider)
            .ok_or_else(|| ModelInfoError::Unknown {
                model: model.to_owned(),
            })
    }
    fn resolved_openai_api_base(&self, api_base: Option<&str>) -> String {
        resolve_openai_api_base(api_base, self.litellm_api_base.as_deref(), &self.openai)
    }
    fn foundry_supports_responses(&self, model: &str, api_base: Option<&str>) -> bool {
        self.foundry.supports_responses(model, api_base)
    }
    fn foundry_chat_rejects_tools(
        &self,
        model: &str,
        reasoning_effort: Option<&ReasoningEffort>,
    ) -> bool {
        self.foundry.chat_rejects_tools(model, reasoning_effort)
    }
    fn is_gpt_5_model(&self, model: &str) -> bool {
        is_gpt_5_model(model)
    }
    fn is_gpt_5_4_plus_model(&self, model: &str) -> bool {
        is_model_gpt_5_4_plus_model(model)
    }
    fn is_gpt_5_search_model(&self, model: &str) -> bool {
        is_model_gpt_5_search_model(model)
    }
}
// OpenAIResponsesAPIConfig._is_gpt_5_model: a pass-through name from another provider is not one
#[must_use]
pub fn is_gpt_5_model(model: &str) -> bool {
    let mut parts = model.split('/');
    let first = parts.next().unwrap_or(model);
    if parts.next().is_some() && first != "openai" {
        return false;
    }
    let normalized = model.rsplit('/').next().unwrap_or(model);
    (model.contains("gpt-5") || model.contains("gpt-6")) && !normalized.contains("gpt-5-chat")
}
