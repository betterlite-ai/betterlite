use betterlite_config::sections::content_filter_records::ContentFilterDetection;

use crate::content_filter_actions::error::ContentFilterActionError;
use crate::content_filter_matching::CategoryMatch;

use super::Matched;

pub fn handle_category_keyword_match(
    matched: &CategoryMatch,
    text: &str,
    keyword_redaction_tag: &str,
    detections: Option<&mut Vec<ContentFilterDetection>>,
) -> Result<String, ContentFilterActionError> {
    let state = Matched::CategoryKeyword {
        found: matched,
        text,
        redaction_tag: keyword_redaction_tag,
    };
    state.record(detections);
    state.finish()
}
