use betterlite_prompts::arguments::json::Object;

use crate::hosted_platform_dispatch::call::EmbeddingCall;

use super::context::{EmbeddingContext, first_secret};
use super::embedding::embedding_call;
use super::error::DispatchError;

const VOLCENGINE_MISSING_KEY: &str = "Missing API key for Volcengine. Set ARK_API_KEY or VOLCENGINE_API_KEY environment variable or pass api_key parameter.";

// the volcengine arm: the passed key, litellm.api_key, ARK_API_KEY, VOLCENGINE_API_KEY; extra_headers or an empty map; empty litellm_params
pub(super) fn volcengine(context: &EmbeddingContext<'_>) -> Result<EmbeddingCall, DispatchError> {
    let mut call = embedding_call(context);
    call.api_key = Some(
        first_secret([
            context.api_key,
            context.settings.api_key.as_ref(),
            context.settings.environment.ark_api_key.as_ref(),
            context.settings.environment.volcengine_api_key.as_ref(),
        ])
        .ok_or(DispatchError::MissingKey {
            message: VOLCENGINE_MISSING_KEY,
        })?,
    );
    call.headers = Some(context.extra_headers.cloned().unwrap_or_default());
    call.litellm_params = Object::new();
    Ok(call)
}
