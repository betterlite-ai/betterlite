use super::error::CompletionCallError;
use super::ports::{
    LocalTokenCountRequest, LocalTokenCounter, ProviderTokenCountRequest, ProviderTokenCounter,
};
use crate::completion_request::ports::{ModelMetadata, ProviderLookup};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
pub const LOCAL_TOKENIZER: &str = "local_tokenizer";
// litellm.types.utils.TokenCountResponse
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TokenCountResponse {
    pub total_tokens: u64,
    pub request_model: String,
    pub model_used: String,
    pub tokenizer_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_response: Option<Json>,
    #[serde(default)]
    pub error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_code: Option<u16>,
}
impl TokenCountResponse {
    // the fallback acount_tokens builds when no provider counter answered
    #[must_use]
    pub fn local(total_tokens: usize, request_model: &str, model_used: &str) -> Self {
        Self {
            total_tokens: u64::try_from(total_tokens).unwrap_or(u64::MAX),
            request_model: request_model.to_owned(),
            model_used: model_used.to_owned(),
            tokenizer_type: String::from(LOCAL_TOKENIZER),
            original_response: None,
            error: false,
            error_message: None,
            status_code: None,
        }
    }
}
pub struct TokenCountRequest<'request> {
    pub model: &'request str,
    pub messages: Option<&'request [Json]>,
    pub tools: Option<&'request [Json]>,
    pub system: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
}
// acount_tokens: the provider's count when it works, B196's local count otherwise
pub async fn count_tokens(
    request: &TokenCountRequest<'_>,
    metadata: &dyn ModelMetadata,
    counter: Option<&dyn ProviderTokenCounter>,
    local: &dyn LocalTokenCounter,
) -> Result<TokenCountResponse, CompletionCallError> {
    let resolved = metadata.resolve_provider(&ProviderLookup {
        model: request.model,
        custom_llm_provider: None,
        api_base: request.api_base,
        api_key: request.api_key,
    })?;
    let provider = resolved.provider.slug().to_owned();
    let api_key = request.api_key.or(resolved.api_key.as_ref());
    let api_base = request.api_base.or(resolved.api_base.as_deref());
    if let Some(counter) = counter.filter(|counter| counter.should_use(&provider))
        && let Some(result) = counter
            .count(&ProviderTokenCountRequest {
                model_to_use: &resolved.model,
                request_model: request.model,
                messages: request.messages,
                tools: request.tools,
                system: request.system,
                api_key,
                api_base,
            })
            .await
        && !result.error
    {
        return Ok(result);
    }
    let mut messages: Vec<Json> = request.messages.map(<[Json]>::to_vec).unwrap_or_default();
    if let Some(system) = request.system.filter(|_given| !messages.is_empty()) {
        messages.insert(0, system_message(system));
    }
    let total = local
        .count(&LocalTokenCountRequest {
            model: request.model,
            messages: &messages,
            tools: request.tools,
        })
        .await;
    let answer = TokenCountResponse::local(total, request.model, &resolved.model);
    Ok(answer)
}
fn system_message(system: &str) -> Json {
    Json::Object(IndexMap::from([
        (String::from("role"), Json::Text(String::from("system"))),
        (String::from("content"), Json::Text(system.to_owned())),
    ]))
}
