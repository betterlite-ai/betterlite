const TOOL_USE_ID_FALLBACK: &str = "tool_use_id";

// LiteLLM rewrites every character outside the caller's set, so a non-empty id always stays non-empty
#[must_use]
pub fn replace_invalid_id_chars(tool_use_id: &str, is_invalid: fn(char) -> bool) -> String {
    if tool_use_id.is_empty() {
        return TOOL_USE_ID_FALLBACK.to_owned();
    }
    tool_use_id
        .chars()
        .map(|candidate| {
            if is_invalid(candidate) {
                '_'
            } else {
                candidate
            }
        })
        .collect()
}

#[must_use]
pub fn sanitize_anthropic_tool_use_id(tool_use_id: &str) -> String {
    replace_invalid_id_chars(tool_use_id, |candidate| {
        !(candidate.is_ascii_alphanumeric() || candidate == '_' || candidate == '-')
    })
}
