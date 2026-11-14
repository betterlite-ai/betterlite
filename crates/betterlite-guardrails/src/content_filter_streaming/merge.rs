use std::cmp::Ordering;

use betterlite_config::sections::content_filter_records::{
    BlockedWordDetection, CategoryKeywordDetection, CompetitorIntentDetection,
    ContentFilterDetection, PatternDetection,
};

mod tests;

// Python keeps the first of each detection by value, and these types carry no PartialEq
fn pattern_detections_equal(left: &PatternDetection, right: &PatternDetection) -> bool {
    left.r#type == right.r#type
        && left.pattern_name == right.pattern_name
        && left.action == right.action
}

fn blocked_word_detections_equal(
    left: &BlockedWordDetection,
    right: &BlockedWordDetection,
) -> bool {
    left.r#type == right.r#type
        && left.keyword == right.keyword
        && left.action == right.action
        && left.description == right.description
}

fn category_keyword_detections_equal(
    left: &CategoryKeywordDetection,
    right: &CategoryKeywordDetection,
) -> bool {
    left.r#type == right.r#type
        && left.category == right.category
        && left.keyword == right.keyword
        && left.severity == right.severity
        && left.action == right.action
}

fn competitor_intent_detections_equal(
    left: &CompetitorIntentDetection,
    right: &CompetitorIntentDetection,
) -> bool {
    left.r#type == right.r#type
        && left.intent == right.intent
        && matches!(
            left.confidence.partial_cmp(&right.confidence),
            Some(Ordering::Equal)
        )
        && left.action_hint == right.action_hint
        && left.entities == right.entities
        && left.signals == right.signals
        && left.evidence.len() == right.evidence.len()
        && left
            .evidence
            .iter()
            .zip(&right.evidence)
            .all(|(left_entry, right_entry)| {
                left_entry.r#type == right_entry.r#type
                    && left_entry.key == right_entry.key
                    && left_entry.value == right_entry.value
                    && left_entry.r#match == right_entry.r#match
            })
}

fn detections_equal(left: &ContentFilterDetection, right: &ContentFilterDetection) -> bool {
    match (left, right) {
        (ContentFilterDetection::Pattern(left), ContentFilterDetection::Pattern(right)) => {
            pattern_detections_equal(left, right)
        }
        (ContentFilterDetection::BlockedWord(left), ContentFilterDetection::BlockedWord(right)) => {
            blocked_word_detections_equal(left, right)
        }
        (
            ContentFilterDetection::CategoryKeyword(left),
            ContentFilterDetection::CategoryKeyword(right),
        ) => category_keyword_detections_equal(left, right),
        (
            ContentFilterDetection::CompetitorIntent(left),
            ContentFilterDetection::CompetitorIntent(right),
        ) => competitor_intent_detections_equal(left, right),
        // a pattern detection never equals a detection of another kind
        _ => false,
    }
}

#[must_use]
pub fn merge_detections(detections: &[ContentFilterDetection]) -> Vec<ContentFilterDetection> {
    let mut merged: Vec<ContentFilterDetection> = Vec::new();
    for detection in detections {
        if merged.iter().any(|kept| detections_equal(kept, detection)) {
            continue;
        }
        merged.push(detection.clone());
    }
    merged
}
