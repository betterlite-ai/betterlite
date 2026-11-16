pub const CUSTOM_LLM_PROVIDER: &str = "bedrock";

// LiteLLM's beta_headers_provider is `self.custom_llm_provider or "bedrock"`.
#[must_use]
pub const fn beta_headers_provider(custom_llm_provider: Option<&str>) -> &str {
    match custom_llm_provider {
        Some(value) if !value.is_empty() => value,
        Some(_) | None => CUSTOM_LLM_PROVIDER,
    }
}
