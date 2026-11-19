use betterlite_config::sections::copilot_chat::Settings;
use betterlite_openai::copilot::headers::{DEFAULT_COPILOT_API_BASE, copilot_default_headers};
use betterlite_openai::openai_base_config::Headers;
use secrecy::ExposeSecret as _;

use crate::api::copilot_chat::credentials::CopilotCredentials;

pub mod error;
pub mod reply;
pub mod request;
mod tests;

use error::CopilotEmbeddingError;

// LiteLLM's GithubCopilotEmbeddingConfig: Copilot credentials and the loaded base, with no OpenAI policy underneath
pub struct EmbeddingConfig<A> {
    pub credentials: A,
    pub settings: Settings,
}

impl<A> EmbeddingConfig<A>
where
    A: CopilotCredentials + Send,
{
    // caller headers replace the Copilot defaults on an exact key, and a falsey key is an AuthenticationError
    pub async fn validate_environment(
        &mut self,
        headers: Headers,
        model: &str,
    ) -> Result<Headers, CopilotEmbeddingError> {
        let key = match self.credentials.api_key().await {
            Ok(Some(key)) if !key.expose_secret().is_empty() => key,
            Ok(None | Some(_)) => {
                return Err(CopilotEmbeddingError::MissingKey {
                    model: model.to_owned(),
                });
            }
            Err(source) => {
                return Err(CopilotEmbeddingError::Authentication {
                    model: model.to_owned(),
                    source,
                });
            }
        };
        let mut merged = copilot_default_headers(&key, None);
        merged.extend(headers);
        Ok(merged)
    }

    // explicit base, then the authenticated endpoint, then GITHUB_COPILOT_API_BASE, then the public API
    pub async fn get_complete_url(&mut self, api_base: Option<&str>) -> String {
        let base = if let Some(explicit) = api_base {
            explicit.to_owned()
        } else if let Some(cached) = self.credentials.api_base().await {
            cached
        } else {
            self.settings
                .api_base
                .clone()
                .unwrap_or_else(|| String::from(DEFAULT_COPILOT_API_BASE))
        };
        format!("{}/embeddings", base.trim_end_matches('/'))
    }
}
