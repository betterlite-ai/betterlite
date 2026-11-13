use betterlite_anthropic::credentials::AnthropicSettings;

pub const MESSAGES_PATH: &str = "/v1/messages";

// B567 already resolved ANTHROPIC_API_BASE over ANTHROPIC_BASE_URL over the default into settings.api_base
#[must_use]
pub fn messages_url(api_base: Option<&str>, settings: &AnthropicSettings) -> String {
    let base = api_base
        .filter(|value| !value.is_empty())
        .unwrap_or(&settings.api_base);
    if base.ends_with(MESSAGES_PATH) {
        return base.to_owned();
    }
    format!("{base}{MESSAGES_PATH}")
}
