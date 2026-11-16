const ANTHROPIC_SEGMENT: &str = "/anthropic";
const COUNT_TOKENS_PATH: &str = "/v1/messages/count_tokens";

// LiteLLM tests the whole base for /anthropic, so a query string can suppress the segment too
#[must_use]
pub fn get_count_tokens_endpoint(api_base: &str) -> String {
    let trimmed = api_base.trim_end_matches('/');
    let resource = if trimmed.contains(ANTHROPIC_SEGMENT) {
        trimmed.to_owned()
    } else {
        format!("{trimmed}{ANTHROPIC_SEGMENT}")
    };
    format!("{resource}{COUNT_TOKENS_PATH}")
}
