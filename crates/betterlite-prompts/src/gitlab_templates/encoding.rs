pub const GITLAB_PREFIX: &str = "gitlab::";

#[must_use]
pub fn encode_prompt_id(raw_id: &str) -> String {
    if raw_id.starts_with(GITLAB_PREFIX) {
        return raw_id.to_owned();
    }
    format!("{GITLAB_PREFIX}{}", raw_id.replace('/', "::"))
}

#[must_use]
pub fn decode_prompt_id(encoded_id: &str) -> String {
    encoded_id
        .strip_prefix(GITLAB_PREFIX)
        .map_or_else(|| encoded_id.to_owned(), |path| path.replace("::", "/"))
}

mod tests;
