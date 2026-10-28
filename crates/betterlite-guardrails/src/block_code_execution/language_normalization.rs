use std::collections::BTreeSet;

use betterlite_config::sections::content_filter_records::CodeBlockActionTaken;

pub(crate) const MASK_PLACEHOLDER: &str = "[CODE_BLOCK_REDACTED]";

pub(crate) const NON_EXECUTABLE_TAGS: &[&str] = &[
    "text",
    "plaintext",
    "plain",
    "markdown",
    "md",
    "output",
    "result",
];

fn normalize_language(tag: &str) -> String {
    let normalized = tag.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "js" => "javascript".to_owned(),
        "py" => "python".to_owned(),
        "sh" => "bash".to_owned(),
        "ts" => "typescript".to_owned(),
        _ => normalized,
    }
}

pub(crate) fn is_blocked_language(
    tag: &str,
    blocked_languages: Option<&[String]>,
    block_all: bool,
) -> bool {
    if block_all {
        return true;
    }
    let normalized = normalize_language(tag);
    blocked_languages.is_some_and(|languages| {
        languages
            .iter()
            .map(|language| normalize_language(language))
            .any(|language| language == normalized)
    })
}

#[derive(Clone, Copy)]
pub(crate) enum ConfidenceMode {
    Explicit,
    All,
    None,
}

pub(crate) fn confidence_for_block(tag: &str, mode: ConfidenceMode) -> f64 {
    if matches!(mode, ConfidenceMode::Explicit) {
        return 1.0;
    }
    if matches!(mode, ConfidenceMode::All) {
        let normalized = normalize_language(tag);
        if NON_EXECUTABLE_TAGS.contains(&normalized.as_str()) {
            return 0.5;
        }
        return 1.0;
    }
    0.0
}

pub(crate) fn normalized_blocked_languages(languages: Option<&[String]>) -> BTreeSet<String> {
    languages
        .unwrap_or_default()
        .iter()
        .map(|language| normalize_language(language))
        .collect()
}

pub(crate) fn action_for_confidence(
    blocked: bool,
    confidence: f64,
    threshold: f64,
) -> CodeBlockActionTaken {
    if !blocked {
        CodeBlockActionTaken::Allow
    } else if confidence >= threshold {
        CodeBlockActionTaken::Block
    } else {
        CodeBlockActionTaken::LogOnly
    }
}
