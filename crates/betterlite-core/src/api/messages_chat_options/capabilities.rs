use crate::api::messages_chat_options::context::OptionsContext;

// PROVIDERS_PROXYING_AN_UNKNOWN_BACKEND: the proxy resolves the real target itself, so a key derived here has no guarantee.
const PROXIES_AN_UNKNOWN_BACKEND: &str = "litellm_proxy";
const PROMPT_CACHE_KEY: &str = "prompt_cache_key";
pub(super) const REASONING_EFFORT: &str = "reasoning_effort";

#[must_use]
pub fn supports_prompt_cache_key(context: &OptionsContext<'_>) -> bool {
    context.custom_llm_provider != Some(PROXIES_AN_UNKNOWN_BACKEND)
        && declares(context, PROMPT_CACHE_KEY)
}

// LiteLLM asks the target itself; the provider is the one the caller named, so an absent one asks nothing.
pub(super) fn declares(context: &OptionsContext<'_>, name: &str) -> bool {
    let Some(provider) = context.custom_llm_provider else {
        return false;
    };
    if context.model.is_empty() {
        return false;
    }
    let declared = match context
        .parameters
        .supported_openai_params(context.model, Some(provider))
    {
        Ok(names) => names,
        Err(never) => match never {},
    };
    declared.is_some_and(|names| names.iter().any(|candidate| candidate == name))
}
