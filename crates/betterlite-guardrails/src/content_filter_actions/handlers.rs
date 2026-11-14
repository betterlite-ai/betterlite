use std::ops::Range;

use betterlite_config::sections::content_filter_records::{
    BlockedWord, BlockedWordDetection, CategoryKeywordDetection, ContentFilterAction,
    ContentFilterDetection, PatternDetection,
};

use crate::content_filter_matching::CategoryMatch;
use crate::pattern_matching::CompiledPattern;
use crate::pattern_matching::masking::{keyword_pattern, mask_spans, redaction_tag};
use crate::primitives::pattern_matching::replace_all;

use super::error::{ContentFilterActionError, ContentFilterBlockDetail};

// Python's re.IGNORECASE, the flag value replace_all reads.
const IGNORECASE: i64 = 2;

const fn action_value(action: ContentFilterAction) -> &'static str {
    match action {
        ContentFilterAction::Block => "BLOCK",
        ContentFilterAction::Mask => "MASK",
    }
}

fn append_category_keyword(
    matched: &CategoryMatch,
    detections: Option<&mut Vec<ContentFilterDetection>>,
) {
    if let Some(list) = detections {
        list.push(ContentFilterDetection::CategoryKeyword(
            CategoryKeywordDetection {
                r#type: "category_keyword".to_owned(),
                category: matched.category.clone(),
                keyword: matched.matched.clone(),
                severity: matched.severity.clone(),
                action: action_value(matched.action).to_owned(),
            },
        ));
    }
}

pub fn handle_conditional_match(
    matched: &CategoryMatch,
    detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<(), ContentFilterActionError> {
    append_category_keyword(matched, detections);
    match matched.action {
        ContentFilterAction::Block => Err(block_conditional_match(matched)),
        ContentFilterAction::Mask => {
            tracing::warn!(
                category = %matched.category,
                matched_phrase = %matched.matched,
                "conditional match detected but MASK action is not supported for conditional categories"
            );
            Ok(())
        }
    }
}

pub fn handle_category_keyword_match(
    matched: &CategoryMatch,
    text: &str,
    keyword_redaction_tag: &str,
    detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<String, ContentFilterActionError> {
    append_category_keyword(matched, detections);
    match matched.action {
        ContentFilterAction::Block => Err(block_category_keyword(matched)),
        ContentFilterAction::Mask => mask_category_keyword(matched, text, keyword_redaction_tag),
    }
}

pub fn handle_pattern_match(
    pattern: &CompiledPattern,
    text: &str,
    spans: &[Range<usize>],
    pattern_redaction_format: &str,
    detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<String, ContentFilterActionError> {
    if let Some(list) = detections {
        list.push(ContentFilterDetection::Pattern(PatternDetection {
            r#type: "pattern".to_owned(),
            pattern_name: pattern.pattern_name.clone(),
            action: action_value(pattern.action).to_owned(),
        }));
    }
    match pattern.action {
        ContentFilterAction::Block => Err(block_pattern_match(pattern)),
        ContentFilterAction::Mask => {
            mask_pattern_match(pattern, text, spans, pattern_redaction_format)
        }
    }
}

pub fn handle_blocked_word_match(
    word: &BlockedWord,
    text: &str,
    keyword_redaction_tag: &str,
    detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<String, ContentFilterActionError> {
    tracing::debug!(
        keyword = %word.keyword,
        action = %action_value(word.action),
        "blocked word found"
    );
    if let Some(list) = detections {
        list.push(ContentFilterDetection::BlockedWord(BlockedWordDetection {
            r#type: "blocked_word".to_owned(),
            keyword: word.keyword.clone(),
            action: action_value(word.action).to_owned(),
            description: word.description.clone(),
        }));
    }
    match word.action {
        ContentFilterAction::Block => Err(block_blocked_word(word)),
        ContentFilterAction::Mask => mask_blocked_word(word, text, keyword_redaction_tag),
    }
}

fn block_conditional_match(matched: &CategoryMatch) -> ContentFilterActionError {
    let error = format!(
        "Content blocked: {} conditional match '{}' detected (severity: {})",
        matched.category, matched.matched, matched.severity
    );
    tracing::warn!(%error, "content filter blocked a conditional match");
    ContentFilterActionError::Blocked {
        detail: ContentFilterBlockDetail::Conditional {
            error,
            category: matched.category.clone(),
            matched_phrase: matched.matched.clone(),
            severity: matched.severity.clone(),
        },
    }
}

fn block_category_keyword(matched: &CategoryMatch) -> ContentFilterActionError {
    let error = format!(
        "Content blocked: {} category keyword '{}' detected (severity: {})",
        matched.category, matched.matched, matched.severity
    );
    tracing::warn!(%error, "content filter blocked a category keyword");
    ContentFilterActionError::Blocked {
        detail: ContentFilterBlockDetail::CategoryKeyword {
            error,
            category: matched.category.clone(),
            keyword: matched.matched.clone(),
            severity: matched.severity.clone(),
        },
    }
}

fn mask_category_keyword(
    matched: &CategoryMatch,
    text: &str,
    keyword_redaction_tag: &str,
) -> Result<String, ContentFilterActionError> {
    let masked = replace_all(
        text,
        &keyword_pattern(&matched.matched),
        keyword_redaction_tag,
        IGNORECASE,
    )?;
    tracing::info!(
        category = %matched.category,
        keyword = %matched.matched,
        severity = %matched.severity,
        "masked category keyword"
    );
    Ok(masked)
}

fn block_pattern_match(pattern: &CompiledPattern) -> ContentFilterActionError {
    let error = format!("Content blocked: {} pattern detected", pattern.pattern_name);
    tracing::warn!(%error, "content filter blocked a pattern match");
    ContentFilterActionError::Blocked {
        detail: ContentFilterBlockDetail::Pattern {
            error,
            pattern: pattern.pattern_name.clone(),
        },
    }
}

fn mask_pattern_match(
    pattern: &CompiledPattern,
    text: &str,
    spans: &[Range<usize>],
    pattern_redaction_format: &str,
) -> Result<String, ContentFilterActionError> {
    let tag = redaction_tag(pattern_redaction_format, &pattern.pattern_name)?;
    let masked = mask_spans(text, spans, &tag)?;
    tracing::info!(pattern = %pattern.pattern_name, "masked every pattern match");
    Ok(masked)
}

fn block_blocked_word(word: &BlockedWord) -> ContentFilterActionError {
    let error = match &word.description {
        Some(description) => format!(
            "Content blocked: keyword '{}' detected ({description})",
            word.keyword
        ),
        None => format!("Content blocked: keyword '{}' detected", word.keyword),
    };
    tracing::warn!(%error, "content filter blocked a word");
    ContentFilterActionError::Blocked {
        detail: ContentFilterBlockDetail::BlockedWord {
            error,
            keyword: word.keyword.clone(),
            description: word.description.clone(),
        },
    }
}

fn mask_blocked_word(
    word: &BlockedWord,
    text: &str,
    keyword_redaction_tag: &str,
) -> Result<String, ContentFilterActionError> {
    let masked = replace_all(
        text,
        &keyword_pattern(&word.keyword),
        keyword_redaction_tag,
        IGNORECASE,
    )?;
    tracing::info!(keyword = %word.keyword, "masked blocked word");
    Ok(masked)
}
