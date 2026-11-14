use betterlite_config::sections::content_filter_records::{BlockedWord, ContentFilterDetection};

use crate::content_filter_actions::error::ContentFilterActionError;

use super::Matched;
use super::action_value;

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
    let state = Matched::BlockedWord {
        word,
        text,
        redaction_tag: keyword_redaction_tag,
    };
    state.record(detections);
    state.finish()
}
