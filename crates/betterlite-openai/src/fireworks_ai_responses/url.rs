use super::settings::FireworksResponsesEnv;

pub const DEFAULT_API_BASE: &str = "https://api.fireworks.ai/inference/v1";

// LiteLLM's get_complete_url: the argument, then FIREWORKS_API_BASE, then the default, trailing slashes trimmed.
#[must_use]
pub fn get_complete_url(api_base: Option<&str>, env: &FireworksResponsesEnv) -> String {
    let from_env = env.api_base.as_deref().filter(|base| !base.is_empty());
    let base = api_base
        .filter(|base| !base.is_empty())
        .or(from_env)
        .unwrap_or(DEFAULT_API_BASE);
    let trimmed = base.trim_end_matches('/');
    format!("{trimmed}/responses")
}
