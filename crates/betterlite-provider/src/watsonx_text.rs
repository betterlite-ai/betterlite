pub mod error;
pub mod reply;
pub mod request;
pub mod stream;

mod tests;

use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::watsonx::messages_to_prompt;
use chrono::{DateTime, Utc};

use self::error::WatsonxTextError;
use self::request::TextGenerationBody;

// IBMWatsonXAIConfig with its dependencies injected; the clock serves both the prompt templates and an absent created_at
pub struct WatsonxTextConfig<'env> {
    pub environment: &'env WatsonxEnvironment,
    pub prompts: &'env PromptFactory,
    pub now: DateTime<Utc>,
}

impl WatsonxTextConfig<'_> {
    // async_transform_request and transform_request share _build_request_payload; betterlite's template call is async
    pub async fn transform_request(
        &self,
        model: &str,
        messages: &[Message],
        options: WatsonxParams,
    ) -> Result<TextGenerationBody, WatsonxTextError> {
        let provider = model
            .split_once('/')
            .map_or(model, |(provider, _)| provider);
        let prompt = messages_to_prompt(self.prompts, model, messages, provider, self.now).await?;
        request::build_body(model, prompt.as_deref(), options, self.environment)
    }
}
