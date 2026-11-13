use std::future::Future;
use std::time::Duration;

use betterlite_anthropic::skills::config::BaseSkillsApiConfig;
use betterlite_anthropic::skills::error::SkillsError;
use betterlite_anthropic::skills::wire::{DeleteSkillResponse, Skill};
use betterlite_openai::error::ProviderError;
use http::header::HeaderMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillsHandlerError {
    #[error("the provider answered with HTTP {status_code}: {message}")]
    Provider {
        status_code: u16,
        message: String,
        status_code_is_synthesized: bool,
        body_code: Option<String>,
        body_message: Option<String>,
    },
    #[error(transparent)]
    Skills(#[from] SkillsError),
}

impl From<ProviderError> for SkillsHandlerError {
    fn from(source: ProviderError) -> Self {
        let body_code = source
            .body
            .as_ref()
            .and_then(|body| body.get("code"))
            .and_then(|code| code.as_str())
            .map(str::to_owned);
        let body_message = source
            .body
            .as_ref()
            .and_then(|body| body.get("message"))
            .and_then(|message| message.as_str())
            .map(str::to_owned);
        Self::Provider {
            status_code: source.status_code,
            message: source.message,
            status_code_is_synthesized: source.status_code_is_synthesized,
            body_code,
            body_message,
        }
    }
}

// the inputs BaseLLMHTTPHandler.get_skill_handler and delete_skill_handler take from the operation
pub struct SkillHttpRequest<'request> {
    pub url: String,
    pub headers: HeaderMap,
    pub timeout: Duration,
    pub config: &'request dyn BaseSkillsApiConfig,
    pub custom_llm_provider: &'request str,
}

pub trait SkillsHttpHandler: Send + Sync {
    fn get_skill(
        &self,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send;

    fn delete_skill(
        &self,
        request: SkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<DeleteSkillResponse, SkillsHandlerError>> + Send;
}
