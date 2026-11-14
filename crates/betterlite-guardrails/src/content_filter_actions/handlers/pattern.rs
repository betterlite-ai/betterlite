use std::ops::Range;

use betterlite_config::sections::content_filter_records::ContentFilterDetection;

use crate::content_filter_actions::error::ContentFilterActionError;
use crate::pattern_matching::CompiledPattern;

use super::Matched;

pub fn handle_pattern_match(
    pattern: &CompiledPattern,
    text: &str,
    spans: &[Range<usize>],
    pattern_redaction_format: &str,
    detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<String, ContentFilterActionError> {
    let state = Matched::Pattern {
        pattern,
        spans,
        text,
        redaction_format: pattern_redaction_format,
    };
    state.record(detections);
    state.finish()
}
