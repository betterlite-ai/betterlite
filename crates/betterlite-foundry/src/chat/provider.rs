use std::collections::BTreeSet;

use betterlite_models::identity::provider_id::ProviderId;
use secrecy::SecretString;

use crate::credentials::host::is_foundry_base;

#[derive(Debug, Clone, Copy)]
pub struct OpenAiModelLists<'data> {
    pub chat: &'data BTreeSet<String>,
    pub text: &'data BTreeSet<String>,
    pub embedding: &'data BTreeSet<String>,
}

// environment_base and environment_key are AZURE_AI_API_BASE and AZURE_AI_API_KEY, read by the
// caller, where an empty variable counts as unset
pub struct ProviderRequest<'data> {
    pub model: &'data str,
    pub api_base: Option<&'data str>,
    pub api_key: Option<&'data SecretString>,
    pub custom_llm_provider: ProviderId,
    pub environment_base: Option<&'data str>,
    pub environment_key: Option<&'data SecretString>,
    pub lists: OpenAiModelLists<'data>,
}

#[derive(Debug, Clone)]
pub struct ProviderInfo {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub custom_llm_provider: ProviderId,
}

#[must_use]
pub fn is_azure_openai_model(
    model: &str,
    api_base: Option<&str>,
    lists: OpenAiModelLists<'_>,
) -> bool {
    let Some(base) = api_base else {
        return false;
    };
    if is_foundry_base(base) {
        return false;
    }
    let stripped = model.split_once('/').map_or(model, |(_, rest)| rest);
    lists.chat.contains(stripped)
        || lists.text.contains(stripped)
        || lists.embedding.contains(stripped)
}

#[must_use]
pub fn provider_info(request: &ProviderRequest<'_>) -> ProviderInfo {
    let resolved_base = request
        .api_base
        .or(request.environment_base)
        .map(str::to_owned);
    let resolved_key = request.api_key.or(request.environment_key).cloned();
    let redirects = is_azure_openai_model(request.model, resolved_base.as_deref(), request.lists);
    ProviderInfo {
        api_base: resolved_base,
        api_key: resolved_key,
        custom_llm_provider: if redirects {
            ProviderId::Azure
        } else {
            request.custom_llm_provider
        },
    }
}
