use std::ops::Range;

use betterlite_config::sections::content_filter_records::BlockedWord;

use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_matching::CategoryMatch;
use crate::pattern_matching::CompiledPattern;
use crate::pattern_matching::masking::{keyword_pattern, mask_spans, redaction_tag};
use crate::primitives::pattern_matching::replace_all;

// Python's re.IGNORECASE, the flag value replace_all reads.
const IGNORECASE: i64 = 2;

pub(super) fn mask_category_keyword(
    found: &CategoryMatch,
    text: &str,
    keyword_redaction_tag: &str,
) -> Result<String, ContentFilterActionError> {
    let masked = replace_all(
        text,
        &keyword_pattern(&found.matched),
        keyword_redaction_tag,
        IGNORECASE,
    )?;
    tracing::info!(
        category = %found.category,
        keyword = %found.matched,
        severity = %found.severity,
        "masked category keyword"
    );
    Ok(masked)
}

pub(super) fn mask_pattern_match(
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

pub(super) fn mask_blocked_word(
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
