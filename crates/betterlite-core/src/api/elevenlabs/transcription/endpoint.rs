const STT_ENDPOINT_PATH: &str = "/v1/speech-to-text";

// LiteLLM keeps an explicit base even when it is empty, and reads the environment only when it is null
#[must_use]
pub fn complete_url(api_base: Option<&str>, environment_base: &str) -> String {
    format!(
        "{}{STT_ENDPOINT_PATH}",
        api_base.unwrap_or(environment_base).trim_end_matches('/')
    )
}
