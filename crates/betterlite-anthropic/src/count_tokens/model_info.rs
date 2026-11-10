use crate::count_tokens::counter::AnthropicTokenCounter;
use crate::count_tokens::handler::AnthropicCountTokensHandler;
use crate::credentials::AnthropicSettings;

#[derive(Debug, Clone)]
pub struct AnthropicModelInfo {
    pub settings: AnthropicSettings,
}

impl AnthropicModelInfo {
    // LiteLLM's factory picks no tokenizer from the model, so the settings' key is all the counter needs
    #[must_use]
    pub fn get_token_counter<T>(
        &self,
        handler: AnthropicCountTokensHandler<T>,
    ) -> AnthropicTokenCounter<T> {
        AnthropicTokenCounter {
            handler,
            environment_api_key: self.settings.api_key.clone(),
        }
    }
}
