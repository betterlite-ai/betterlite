pub mod blocked_word;
pub mod category_keyword;
pub mod conditional;
pub mod masking;
pub mod pattern;

use std::ops::Range;

use betterlite_config::sections::content_filter_records::{
    BlockedWord, BlockedWordDetection, CategoryKeywordDetection, ContentFilterAction,
    ContentFilterDetection, PatternDetection,
};

use crate::content_filter_matching::CategoryMatch;
use crate::pattern_matching::CompiledPattern;

use self::masking::{mask_blocked_word, mask_category_keyword, mask_pattern_match};
use super::error::{ContentFilterActionError, ContentFilterBlockDetail};

const fn action_value(action: ContentFilterAction) -> &'static str {
    match action {
        ContentFilterAction::Block => "BLOCK",
        ContentFilterAction::Mask => "MASK",
    }
}

// One match of any detection kind, carrying what its masking action needs.
enum Matched<'matched> {
    Conditional {
        found: &'matched CategoryMatch,
        text: &'matched str,
    },
    CategoryKeyword {
        found: &'matched CategoryMatch,
        text: &'matched str,
        redaction_tag: &'matched str,
    },
    Pattern {
        pattern: &'matched CompiledPattern,
        spans: &'matched [Range<usize>],
        text: &'matched str,
        redaction_format: &'matched str,
    },
    BlockedWord {
        word: &'matched BlockedWord,
        text: &'matched str,
        redaction_tag: &'matched str,
    },
}

impl<'matched> Matched<'matched> {
    const fn action(&self) -> ContentFilterAction {
        match self {
            Self::Conditional { found, .. } | Self::CategoryKeyword { found, .. } => found.action,
            Self::Pattern { pattern, .. } => pattern.action,
            Self::BlockedWord { word, .. } => word.action,
        }
    }

    fn record(&self, detections: Option<&mut Vec<ContentFilterDetection>>) {
        let Some(list) = detections else {
            return;
        };
        match self {
            Self::Conditional { found, .. } | Self::CategoryKeyword { found, .. } => {
                list.push(ContentFilterDetection::CategoryKeyword(
                    CategoryKeywordDetection {
                        r#type: "category_keyword".to_owned(),
                        category: found.category.clone(),
                        keyword: found.matched.clone(),
                        severity: found.severity.clone(),
                        action: action_value(found.action).to_owned(),
                    },
                ));
            }
            Self::Pattern { pattern, .. } => {
                list.push(ContentFilterDetection::Pattern(PatternDetection {
                    r#type: "pattern".to_owned(),
                    pattern_name: pattern.pattern_name.clone(),
                    action: action_value(pattern.action).to_owned(),
                }));
            }
            Self::BlockedWord { word, .. } => {
                list.push(ContentFilterDetection::BlockedWord(BlockedWordDetection {
                    r#type: "blocked_word".to_owned(),
                    keyword: word.keyword.clone(),
                    action: action_value(word.action).to_owned(),
                    description: word.description.clone(),
                }));
            }
        }
    }

    fn finish(&self) -> Result<String, ContentFilterActionError> {
        match self.action() {
            ContentFilterAction::Block => Err(self.blocked()),
            ContentFilterAction::Mask => self.masked(),
        }
    }

    fn blocked(&self) -> ContentFilterActionError {
        let detail = self.detail();
        tracing::warn!(
            error = detail_message(&detail),
            "content filter blocked the text"
        );
        ContentFilterActionError::Blocked { detail }
    }

    fn detail(&self) -> ContentFilterBlockDetail {
        match self {
            Self::Conditional { found, .. } => conditional_detail(found),
            Self::CategoryKeyword { found, .. } => ContentFilterBlockDetail::CategoryKeyword {
                error: format!(
                    "Content blocked: {} category keyword '{}' detected (severity: {})",
                    found.category, found.matched, found.severity
                ),
                category: found.category.clone(),
                keyword: found.matched.clone(),
                severity: found.severity.clone(),
            },
            Self::Pattern { pattern, .. } => ContentFilterBlockDetail::Pattern {
                error: format!("Content blocked: {} pattern detected", pattern.pattern_name),
                pattern: pattern.pattern_name.clone(),
            },
            Self::BlockedWord { word, .. } => blocked_word_detail(word),
        }
    }

    fn masked(&self) -> Result<String, ContentFilterActionError> {
        match self {
            Self::Conditional { found, text } => {
                tracing::warn!(
                    category = %found.category,
                    matched_phrase = %found.matched,
                    "conditional match detected but MASK action is not supported for conditional categories"
                );
                Ok((*text).to_owned())
            }
            Self::CategoryKeyword {
                found,
                text,
                redaction_tag,
            } => mask_category_keyword(found, text, redaction_tag),
            Self::Pattern {
                pattern,
                spans,
                text,
                redaction_format,
            } => mask_pattern_match(pattern, text, spans, redaction_format),
            Self::BlockedWord {
                word,
                text,
                redaction_tag,
            } => mask_blocked_word(word, text, redaction_tag),
        }
    }
}

fn detail_message(detail: &ContentFilterBlockDetail) -> &str {
    match detail {
        ContentFilterBlockDetail::Conditional { error, .. }
        | ContentFilterBlockDetail::CategoryKeyword { error, .. }
        | ContentFilterBlockDetail::Pattern { error, .. }
        | ContentFilterBlockDetail::BlockedWord { error, .. } => error,
    }
}

fn conditional_detail(found: &CategoryMatch) -> ContentFilterBlockDetail {
    ContentFilterBlockDetail::Conditional {
        error: format!(
            "Content blocked: {} conditional match '{}' detected (severity: {})",
            found.category, found.matched, found.severity
        ),
        category: found.category.clone(),
        matched_phrase: found.matched.clone(),
        severity: found.severity.clone(),
    }
}

fn blocked_word_detail(word: &BlockedWord) -> ContentFilterBlockDetail {
    ContentFilterBlockDetail::BlockedWord {
        error: blocked_word_message(word),
        keyword: word.keyword.clone(),
        description: word.description.clone(),
    }
}

fn blocked_word_message(word: &BlockedWord) -> String {
    match &word.description {
        Some(description) => format!(
            "Content blocked: keyword '{}' detected ({description})",
            word.keyword
        ),
        None => format!("Content blocked: keyword '{}' detected", word.keyword),
    }
}
