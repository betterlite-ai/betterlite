use betterlite_config::sections::copilot_chat::Settings;
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use betterlite_openai::copilot::headers::{DEFAULT_COPILOT_API_BASE, copilot_default_headers};
use betterlite_openai::openai_base_config::Headers;
use secrecy::ExposeSecret as _;
use uuid::Uuid;

use crate::api::copilot_chat::credentials::CopilotCredentials;

pub mod error;
pub mod input;
pub mod model_gate;
pub mod stream;

use error::CopilotResponsesError;
use input::{LiteLlmParams, has_vision_input, initiator};
use stream::StreamItemIds;

mod tests;

pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;

// LiteLLM's GithubCopilotResponsesAPIConfig: built fresh per request, so its stream anchors never outlive the request
pub struct ResponsesConfig<A> {
    pub credentials: A,
    pub settings: Settings,
    pub max_recurse_depth: i64,
    pub item_ids: StreamItemIds,
}

impl<A> ResponsesConfig<A>
where
    A: CopilotCredentials + Send,
{
    // caller headers win over the Copilot defaults; only a typed params object's input sets X-Initiator and the vision header
    pub async fn validate_environment(
        &mut self,
        headers: Headers,
        params: &LiteLlmParams<'_>,
        model: &str,
        request_id: Uuid,
    ) -> Result<Headers, CopilotResponsesError> {
        let stored = match self.credentials.api_key().await {
            Ok(stored) => stored,
            Err(source @ ApiKeyError::AccessToken(_)) => {
                return Err(CopilotResponsesError::Login(source));
            }
            Err(source) => {
                return Err(CopilotResponsesError::Authentication {
                    model: model.to_owned(),
                    source,
                });
            }
        };
        let Some(key) = stored.filter(|secret| !secret.expose_secret().is_empty()) else {
            return Err(CopilotResponsesError::MissingKey {
                model: model.to_owned(),
            });
        };
        let mut merged = copilot_default_headers(&key, Some(request_id));
        merged.extend(headers);
        let LiteLlmParams::Typed { input: Some(input) } = params else {
            return Ok(merged);
        };
        merged.insert(String::from("X-Initiator"), String::from(initiator(input)));
        if has_vision_input(input, self.max_recurse_depth) {
            merged.insert(String::from("copilot-vision-request"), String::from("true"));
        }
        Ok(merged)
    }

    // explicit base, then the cached endpoint, then GITHUB_COPILOT_API_BASE, then the public API
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
        format!("{}/responses", base.trim_end_matches('/'))
    }
}
