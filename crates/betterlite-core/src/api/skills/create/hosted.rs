use std::future::Future;

use betterlite_anthropic::skills::wire::Skill;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::skills::get_delete::handler::SkillsHandlerError;
use crate::api::skills::get_delete::hosted::HostedSkillsHandler;
use crate::api::skills::get_delete::principal::user_api_key_auth_from_kwargs;

use super::call::CreateSkillCall;
use super::error::CreateSkillError;
use super::metadata::{hosted_text, requester_metadata};
use super::request::text_field;

pub struct HostedCreateSkillRequest<'request> {
    pub display_title: Option<&'request str>,
    pub description: Option<&'request str>,
    pub instructions: Option<&'request str>,
    pub files: Option<&'request [Json]>,
    pub metadata: Option<&'request IndexMap<String, Json>>,
    pub user_id: Option<&'request str>,
    pub user_api_key_auth: Option<&'request Json>,
    pub litellm_call_id: Option<&'request str>,
}

// LiteLLM's one hosted skills handler serves every operation, so create extends B3391's trait and its lazy cache
pub trait HostedSkillsCreator: HostedSkillsHandler {
    fn create_skill(
        &self,
        request: HostedCreateSkillRequest<'_>,
    ) -> impl Future<Output = Result<Skill, SkillsHandlerError>> + Send;
}

pub fn hosted_request<'call>(
    call: &'call CreateSkillCall<'_>,
) -> Result<HostedCreateSkillRequest<'call>, CreateSkillError> {
    Ok(HostedCreateSkillRequest {
        display_title: call.display_title,
        description: hosted_text(call.kwargs, call.extra_body, "description")?,
        instructions: hosted_text(call.kwargs, call.extra_body, "instructions")?,
        files: call.files,
        metadata: requester_metadata(call.kwargs, call.extra_body),
        user_id: text_field(call.kwargs, "user_id")?,
        user_api_key_auth: user_api_key_auth_from_kwargs(call.kwargs),
        litellm_call_id: call.litellm_call_id,
    })
}
