use std::future::Future;
use std::time::Duration;

use betterlite_anthropic::skills::config::BaseSkillsApiConfig;
use betterlite_anthropic::skills::wire::{CreateSkillRequest, Skill};
use http::header::HeaderMap;

use crate::api::skills::get_delete::handler::SkillsHandlerError;

// the inputs BaseLLMHTTPHandler.create_skill_handler takes from the operation
pub struct CreateSkillHttpRequest<'request> {
    pub url: String,
    pub headers: HeaderMap,
    pub timeout: Duration,
    pub body: &'request CreateSkillRequest,
    pub config: &'request dyn BaseSkillsApiConfig,
    pub custom_llm_provider: &'request str,
}

pub trait SkillsCreateHttpHandler: Send + Sync {
    fn create_skill(
        &self,
        request: CreateSkillHttpRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send;
}
