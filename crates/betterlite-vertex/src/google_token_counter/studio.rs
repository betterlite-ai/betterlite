use betterlite_config::sections::studio_identity::Settings;
use betterlite_models::decisions::Json;

use crate::google_count_tokens::decode::GoogleCountResponse;
use crate::google_count_tokens::error::GoogleCountError;
use crate::google_count_tokens::studio::{StudioCountInput, studio_request};
use crate::google_count_tokens::transport::CountRequest;
use crate::google_token_counter::counter::CountTokensCall;
use crate::google_token_counter::error::CountTokensError;
use crate::google_token_counter::reply::{CountReply, TokenCountResponse, counted};
use crate::studio_identity::NativeParams;

// LiteLLM's GoogleAIStudioTokenCounter.count_tokens: the deployment's contents go as given and messages are never read
pub(crate) fn request(
    settings: &Settings,
    call: &CountTokensCall<'_>,
) -> Result<CountRequest, GoogleCountError> {
    let missing = Json::Null;
    let params = NativeParams {
        api_key: None,
        gemini_api_key: call.deployment.gemini_api_key.cloned(),
    };
    studio_request(&StudioCountInput {
        model: call.model_to_use,
        contents: call.contents.unwrap_or(&missing),
        api_key: call.deployment.api_key,
        api_base: call.deployment.api_base,
        params: &params,
        settings,
        timeout: call.deployment.timeout,
    })
}

// every Studio reply is wrapped, and one without totalTokens counts zero
pub(crate) fn response(
    call: &CountTokensCall<'_>,
    reply: GoogleCountResponse,
) -> Result<TokenCountResponse, CountTokensError> {
    let total = reply.total_tokens.unwrap_or(0);
    let counted = counted(total)?;
    let tokenizer_type = reply.tokenizer_used.clone().unwrap_or_default();
    Ok(TokenCountResponse::new(
        call,
        counted,
        tokenizer_type,
        CountReply::Google(Box::new(reply)),
    ))
}
