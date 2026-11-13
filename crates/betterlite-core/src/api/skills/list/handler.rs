use std::future::Future;
use std::time::Duration;

use betterlite_anthropic::skills::config::BaseSkillsApiConfig;
use betterlite_anthropic::skills::wire::ListSkillsResponse;
use http::header::HeaderMap;

use crate::api::skills::get_delete::handler::SkillsHandlerError;

// the inputs BaseLLMHTTPHandler.list_skills_handler takes from the operation; the async flag is the future a handler returns
pub struct ListSkillsHttpRequest<'request> {
    pub url: String,
    pub query_params: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub timeout: Duration,
    pub config: &'request dyn BaseSkillsApiConfig,
    pub custom_llm_provider: &'request str,
}

pub trait SkillsListHttpHandler: Send + Sync {
    fn list_skills(
        &self,
        request: ListSkillsHttpRequest<'_>,
    ) -> impl Future<Output = Result<ListSkillsResponse, SkillsHandlerError>> + Send;
}
