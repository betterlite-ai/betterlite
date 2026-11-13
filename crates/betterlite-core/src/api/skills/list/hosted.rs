use std::future::Future;

use betterlite_anthropic::skills::wire::ListSkillsResponse;
use betterlite_models::decisions::Json;

use crate::api::skills::get_delete::handler::SkillsHandlerError;
use crate::api::skills::get_delete::hosted::HostedSkillsHandler;

// LiteLLM passes `limit or 20` to the hosted registry
pub const DEFAULT_HOSTED_LIMIT: i64 = 20;

// the hosted list contract has only a resolved limit and a fixed offset: page, source and extra_query never reach it
pub struct HostedListSkillsRequest<'request> {
    pub limit: i64,
    pub offset: i64,
    pub user_api_key_auth: Option<&'request Json>,
    pub litellm_call_id: Option<&'request str>,
}

// LiteLLM's one hosted skills handler serves every operation, so list extends B3391's trait and its lazy cache
pub trait HostedSkillsLister: HostedSkillsHandler {
    fn list_skills(
        &self,
        request: HostedListSkillsRequest<'_>,
    ) -> impl Future<Output = Result<ListSkillsResponse, SkillsHandlerError>> + Send;
}
