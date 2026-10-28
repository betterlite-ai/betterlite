#[cfg(test)]
mod tests;

pub const MASK_PLACEHOLDER: &str = "[CODE_BLOCK_REDACTED]";

const NON_EXECUTABLE_TAGS: [&str; 7] = [
    "text",
    "plaintext",
    "plain",
    "markdown",
    "md",
    "output",
    "result",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageMatch {
    Listed,
    BlockAll,
    Unlisted,
}

#[must_use]
pub fn normalize_language(tag: &str) -> String {
    let lowered = tag.trim().to_lowercase();
    match lowered.as_str() {
        "js" => "javascript".to_owned(),
        "py" => "python".to_owned(),
        "sh" => "bash".to_owned(),
        "ts" => "typescript".to_owned(),
        _ => lowered,
    }
}

// An empty list means LiteLLM's block_all mode, where every tag counts as blocked
#[must_use]
pub fn match_language(tag: &str, blocked_languages: &[String]) -> LanguageMatch {
    if blocked_languages.is_empty() {
        return LanguageMatch::BlockAll;
    }
    let normalized = normalize_language(tag);
    if blocked_languages
        .iter()
        .any(|language| normalize_language(language) == normalized)
    {
        LanguageMatch::Listed
    } else {
        LanguageMatch::Unlisted
    }
}

#[must_use]
pub fn confidence_for_block(tag: &str, language_match: LanguageMatch) -> f64 {
    match language_match {
        LanguageMatch::BlockAll
            if NON_EXECUTABLE_TAGS.contains(&normalize_language(tag).as_str()) =>
        {
            0.5
        }
        LanguageMatch::Listed | LanguageMatch::BlockAll => 1.0,
        LanguageMatch::Unlisted => 0.0,
    }
}
