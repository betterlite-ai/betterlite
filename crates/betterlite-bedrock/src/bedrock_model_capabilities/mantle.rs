pub const MANTLE_MESSAGES_PATH: &str = "/anthropic/v1/messages";

const MANTLE_OPENAI_BASE_SUFFIXES: [&str; 2] = ["/openai/v1", "/v1"];

#[must_use]
pub fn mantle_messages_url(
    api_base: Option<&str>,
    aws_bedrock_runtime_endpoint: Option<&str>,
    mantle_api_base: Option<&str>,
    region: &str,
) -> String {
    let chosen = [api_base, aws_bedrock_runtime_endpoint]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| mantle_api_base.map(mantle_env_base));
    let Some(base) = chosen.filter(|value| !value.is_empty()) else {
        return format!("https://bedrock-mantle.{region}.api.aws{MANTLE_MESSAGES_PATH}");
    };
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with(MANTLE_MESSAGES_PATH) {
        trimmed.to_owned()
    } else {
        format!("{trimmed}{MANTLE_MESSAGES_PATH}")
    }
}

// only the environment-sourced base loses the OpenAI surface suffix the mantle/* routes need
fn mantle_env_base(value: &str) -> String {
    let base = value.trim_end_matches('/');
    MANTLE_OPENAI_BASE_SUFFIXES
        .iter()
        .find_map(|suffix| base.strip_suffix(*suffix))
        .map_or_else(|| base.to_owned(), str::to_owned)
}
