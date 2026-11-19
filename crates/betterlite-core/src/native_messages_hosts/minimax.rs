use betterlite_prompts::arguments::json::Object;

use crate::api::messages_native_transform::capabilities::ThinkingCapabilities;
use crate::api::messages_native_transform::request::{TransformRequest, transform_native_request};

use super::error::HostError;
use super::keys::{BaseSources, resolve_base};
use super::request::HostRequest;

pub const CUSTOM_LLM_PROVIDER: &str = "minimax";
pub const STRIPS_BILLING_METADATA: bool = true;
pub const DEFAULT_API_BASE: &str = "https://api.minimax.io/anthropic/v1/messages";

// MiniMax only adjusts the separator before its suffix, so an existing /v1 is kept and doubled
#[must_use]
pub fn complete_url(sources: BaseSources<'_>) -> String {
    let resolved = resolve_base(sources, DEFAULT_API_BASE);
    if resolved.ends_with("/v1/messages") {
        return resolved;
    }
    if resolved.ends_with('/') {
        return format!("{resolved}v1/messages");
    }
    format!("{resolved}/v1/messages")
}

pub async fn transform_request<Capabilities>(
    capabilities: &mut Capabilities,
    request: HostRequest<'_>,
) -> Result<Object, HostError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    transform_native_request(
        capabilities,
        TransformRequest {
            model: request.model,
            messages: request.messages,
            optional_params: request.optional_params,
            custom_llm_provider: CUSTOM_LLM_PROVIDER,
            strips_billing_metadata: STRIPS_BILLING_METADATA,
        },
    )
    .await
    .map_err(HostError::from)
}
