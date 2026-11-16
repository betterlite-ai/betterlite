use super::BedrockPassthroughPlan;
use super::error::PassthroughFailure;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{PassthroughTarget, RelayedResponse};
use serde::Serialize;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NativeMessage {
    pub role: &'static str,
    pub content: &'static str,
}
pub const LOGGED_MESSAGES: [NativeMessage; 1] = [NativeMessage {
    role: "user",
    content: "no-message-pass-through-endpoint",
}];
pub struct NativeReplyRequest<'request> {
    pub tag: ChatConfigTag,
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub status_code: u16,
    pub body: &'request [u8],
    pub request_data: &'request Object,
    pub messages: &'request [NativeMessage],
    pub optional_params: &'request Object,
    pub litellm_params: &'request Object,
    pub api_key: &'request str,
}
pub trait NativeReplyTransform: Sync {
    fn transform(&self, request: &NativeReplyRequest<'_>) -> Result<Json, Box<ProviderError>>;
}
fn resolved_tag(
    plan: &BedrockPassthroughPlan<'_>,
    target: &PassthroughTarget<'_>,
) -> Result<Option<ChatConfigTag>, PassthroughFailure> {
    let chat_config_model = if target.endpoint.contains("invoke") {
        format!("invoke/{}", target.model)
    } else if target.endpoint.contains("converse") {
        format!("converse/{}", target.model)
    } else {
        return Ok(None);
    };
    let provider = target
        .custom_llm_provider
        .parse::<ProviderId>()
        .map_err(|_error| PassthroughFailure::UnknownProvider {
            provider: target.custom_llm_provider.to_owned(),
        })?;
    let tag = plan
        .selector
        .select_chat_config(&chat_config_model, provider)?;
    tag.map(Some)
        .ok_or_else(|| PassthroughFailure::NoProviderConfig {
            model: target.model.to_owned(),
        })
}
pub fn logged_reply(
    plan: &BedrockPassthroughPlan<'_>,
    target: &PassthroughTarget<'_>,
    response: &RelayedResponse<'_>,
    request_data: &Object,
) -> Result<Option<Json>, PassthroughFailure> {
    let Some(tag) = resolved_tag(plan, target)? else {
        return Ok(None);
    };
    let request = NativeReplyRequest {
        tag,
        model: target.model,
        custom_llm_provider: target.custom_llm_provider,
        status_code: response.status_code,
        body: response.body,
        request_data,
        messages: &LOGGED_MESSAGES,
        optional_params: &Object::new(),
        litellm_params: &Object::new(),
        api_key: "",
    };
    plan.transform
        .transform(&request)
        .map(Some)
        .map_err(|_error| PassthroughFailure::Reply)
}
