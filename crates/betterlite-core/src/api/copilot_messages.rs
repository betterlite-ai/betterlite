use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::copilot::headers::DEFAULT_COPILOT_API_BASE;
use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;

use crate::api::anthropic_beta_filter::native_injection::{NativeRequest, inject_native_betas};
use crate::api::copilot_chat::credentials::CopilotCredentials;

pub mod error;
mod headers;
mod tests;

use error::CopilotMessagesError;

pub type Headers = IndexMap<String, String>;

pub const CUSTOM_LLM_PROVIDER: ProviderId = ProviderId::GithubCopilot;
// Copilot's /v1/messages endpoint runs no web search itself and passes Anthropic betas through unfiltered
pub const HANDLES_WEB_SEARCH_NATIVELY: bool = false;
pub const FILTERS_ANTHROPIC_BETA_HEADERS: bool = false;
// LiteLLM pins the Messages proxy version instead of the shared 2025-04-01 inference header default
pub const API_VERSION: &str = "2026-06-01";
pub const MESSAGES_PATH: &str = "/v1/messages";

pub struct MessagesRequest<'request> {
    pub optional_params: &'request IndexMap<String, Json>,
    pub messages: &'request [Json],
}

#[derive(Debug)]
pub struct MessagesEnvironment {
    pub headers: Headers,
    pub api_base: String,
}

// LiteLLM's GithubCopilotAnthropicMessagesConfig: only the authenticated session host may receive the token
pub struct MessagesConfig<A> {
    pub credentials: A,
}

impl<A> MessagesConfig<A>
where
    A: CopilotCredentials + Send,
{
    pub async fn validate_environment(
        &mut self,
        headers: Headers,
        request: &MessagesRequest<'_>,
        model: &str,
    ) -> Result<MessagesEnvironment, CopilotMessagesError> {
        let cached = self.credentials.api_base().await;
        let key = match self.credentials.api_key().await {
            Ok(key) => key,
            Err(source) => {
                return Err(CopilotMessagesError::Authentication {
                    model: model.to_owned(),
                    source,
                });
            }
        };
        let provider = <&'static str>::from(CUSTOM_LLM_PROVIDER);
        let headers = headers::with_copilot_defaults(headers, key.as_ref());
        let headers = inject_native_betas(
            headers,
            &NativeRequest {
                provider,
                optional_params: request.optional_params,
                messages: request.messages,
            },
        )?;
        Ok(MessagesEnvironment {
            headers,
            api_base: cached
                .as_deref()
                .unwrap_or(DEFAULT_COPILOT_API_BASE)
                .trim_end_matches('/')
                .to_owned(),
        })
    }

    // api_base is the trusted value validate_environment resolved, never the caller's own base
    pub async fn get_complete_url(&mut self, api_base: Option<&str>) -> String {
        let resolved = match api_base.filter(|value| !value.is_empty()) {
            Some(trusted) => trusted.to_owned(),
            None => self
                .credentials
                .api_base()
                .await
                .unwrap_or_else(|| String::from(DEFAULT_COPILOT_API_BASE)),
        };
        let trimmed = resolved.trim_end_matches('/');
        if trimmed.ends_with(MESSAGES_PATH) {
            trimmed.to_owned()
        } else {
            format!("{trimmed}{MESSAGES_PATH}")
        }
    }
}
