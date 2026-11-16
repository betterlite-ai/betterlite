pub mod capabilities;
pub mod environment;
pub mod environment_error;
pub mod error;
pub mod fields;
pub mod messages;
pub mod provider;
pub mod request;
pub mod response;
pub mod sources;
pub mod url;
pub mod url_error;

mod tests;

use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_prompts::message_content::Message;

use environment::EnvironmentRequest;
use error::ChatError;

pub const AZURE_AI_PREFIX: &str = "azure_ai/";

pub struct FoundryChatOptions<'data> {
    pub model: &'data str,
    pub api_version: Option<&'data str>,
    pub messages: &'data [Message],
    pub optional_params: &'data Object,
    pub environment: EnvironmentRequest<'data>,
}

impl FoundryChatOptions<'_> {
    pub fn prepare(&self) -> Result<PreparedChat, ChatError> {
        let api_base = self.environment.api_base;
        let headers = environment::validate_environment(self.environment.clone())?;
        let url = url::get_complete_url(api_base, self.api_version)?;
        let body = request::transform_request(self.model, self.messages, self.optional_params)?;
        Ok(PreparedChat { headers, url, body })
    }
}

// The headers carry the resolved credential, so this type has no Debug impl
pub struct PreparedChat {
    pub headers: Headers,
    pub url: String,
    pub body: Object,
}
