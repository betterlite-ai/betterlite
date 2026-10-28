use betterlite_config::sections::content_filter_records::CodeBlockActionTaken;
use regex::Regex;

use super::language_normalization::{
    action_for_confidence, confidence_for_block, is_blocked_language, normalized_blocked_languages,
};

pub(crate) type CodeBlock = (usize, usize, String, String, f64, CodeBlockActionTaken);

#[expect(
    clippy::too_many_lines,
    reason = "capture mapping keeps fenced-block semantics together"
)]
pub(crate) fn find_blocks(
    text: &str,
    blocked_languages: Option<&[String]>,
    block_all: bool,
    threshold: f64,
) -> Vec<CodeBlock> {
    let normalized_languages = normalized_blocked_languages(blocked_languages);
    let Ok(regex) = Regex::new(r"(?s)```(\w*)\n(.*?)```") else {
        return Vec::new();
    };
    regex
        .captures_iter(text)
        .filter_map(|capture| {
            let whole = capture.get(0)?;
            let tag = capture.get(1).map_or("", |value| value.as_str()).trim();
            let body = capture.get(2).map_or("", |value| value.as_str());
            let tag_in_list = normalized_languages.contains(&tag.to_ascii_lowercase())
                || (tag.eq_ignore_ascii_case("js") && normalized_languages.contains("javascript"))
                || (tag.eq_ignore_ascii_case("py") && normalized_languages.contains("python"))
                || (tag.eq_ignore_ascii_case("sh") && normalized_languages.contains("bash"))
                || (tag.eq_ignore_ascii_case("ts") && normalized_languages.contains("typescript"));
            let blocked = is_blocked_language(tag, blocked_languages, block_all);
            let confidence_mode = if tag_in_list {
                super::language_normalization::ConfidenceMode::Explicit
            } else if block_all {
                super::language_normalization::ConfidenceMode::All
            } else {
                super::language_normalization::ConfidenceMode::None
            };
            let confidence = confidence_for_block(tag, confidence_mode);
            let action = action_for_confidence(blocked, confidence, threshold);
            Some((
                text.char_indices()
                    .take_while(|(index, _)| *index < whole.start())
                    .count(),
                text.char_indices()
                    .take_while(|(index, _)| *index < whole.end())
                    .count(),
                if tag.is_empty() {
                    "(none)".to_owned()
                } else {
                    tag.to_owned()
                },
                body.to_owned(),
                confidence,
                action,
            ))
        })
        .collect()
}
