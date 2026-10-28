#[cfg(test)]
mod tests;

use betterlite_config::sections::content_filter_records::CodeBlockActionTaken;
use regex::Regex;

use super::language_normalization::{LanguageMatch, confidence_for_block, match_language};

pub const FENCED_BLOCK_PATTERN: &str = r"(?s)```(\w*)\n(.*?)```";

#[derive(Debug, Clone, PartialEq)]
pub struct CodeBlock {
    // start and end count characters, as Python's str offsets do
    pub start: usize,
    pub end: usize,
    pub language: String,
    pub body: String,
    pub confidence: f64,
    pub action_taken: CodeBlockActionTaken,
}

#[derive(Debug, Clone)]
pub struct BlockDetector {
    pub fenced_block: Regex,
    pub blocked_languages: Vec<String>,
    pub confidence_threshold: f64,
}

impl BlockDetector {
    #[must_use]
    pub fn find_blocks(&self, text: &str) -> Vec<CodeBlock> {
        let mut blocks = Vec::new();
        let mut scanned_bytes = 0;
        let mut scanned_chars: usize = 0;
        for captures in self.fenced_block.captures_iter(text) {
            let whole = captures.get_match();
            let tag = captures.get(1).map_or("", |group| group.as_str());
            let language_match = match_language(tag, &self.blocked_languages);
            let confidence = confidence_for_block(tag, language_match);
            let action_taken = match language_match {
                LanguageMatch::Unlisted => CodeBlockActionTaken::Allow,
                LanguageMatch::Listed | LanguageMatch::BlockAll
                    if confidence >= self.confidence_threshold =>
                {
                    CodeBlockActionTaken::Block
                }
                LanguageMatch::Listed | LanguageMatch::BlockAll => CodeBlockActionTaken::LogOnly,
            };
            let gap = text
                .get(scanned_bytes..whole.start())
                .map_or(0, |between| between.chars().count());
            let start = scanned_chars.saturating_add(gap);
            let end = start.saturating_add(whole.as_str().chars().count());
            blocks.push(CodeBlock {
                start,
                end,
                language: if tag.is_empty() { "(none)" } else { tag }.to_owned(),
                body: captures
                    .get(2)
                    .map_or("", |group| group.as_str())
                    .to_owned(),
                confidence,
                action_taken,
            });
            scanned_bytes = whole.end();
            scanned_chars = end;
        }
        blocks
    }
}
