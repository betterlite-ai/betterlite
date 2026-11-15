use betterlite_config::sections::content_filter_records::ContentFilterDetection;

use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_matching::CategoryMatch;

use super::Matched;

pub fn handle_conditional_match(
    matched: &CategoryMatch,
    text: &str,
    detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<String, ContentFilterActionError> {
    let state = Matched::Conditional {
        found: matched,
        text,
    };
    state.record(detections);
    state.finish()
}
