use std::cmp::Ordering;

use betterlite_config::sections::content_filter_records::overrides::CompetitorIntentEvidenceEntry;
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

fn category_keyword_equal(
    left: &CategoryKeywordDetection,
    right: &CategoryKeywordDetection,
) -> bool {
    left.r#type == right.r#type
        && left.category == right.category
        && left.keyword == right.keyword
        && left.severity == right.severity
        && left.action == right.action
}

fn intent_detections_equal(
    candidate: &CompetitorIntentDetection,
    kept: &CompetitorIntentDetection,
) -> bool {
    candidate.r#type == kept.r#type
        && candidate.intent == kept.intent
        && matches!(
            candidate.confidence.partial_cmp(&kept.confidence),
            Some(Ordering::Equal)
        )
        && candidate.action_hint == kept.action_hint
        && candidate.entities == kept.entities
        && candidate.signals == kept.signals
        && evidence_equal(&candidate.evidence, &kept.evidence)
}

fn evidence_equal(
    candidate: &[CompetitorIntentEvidenceEntry],
    kept: &[CompetitorIntentEvidenceEntry],
) -> bool {
    candidate.len() == kept.len()
        && candidate.iter().zip(kept).all(|(entry, held)| {
            entry.r#type == held.r#type
                && entry.key == held.key
                && entry.value == held.value
                && entry.r#match == held.r#match
        })
}

fn detections_equal(first: &ContentFilterDetection, second: &ContentFilterDetection) -> bool {
    match (first, second) {
        (ContentFilterDetection::Pattern(left), ContentFilterDetection::Pattern(right)) => {
            pattern_detections_equal(left, right)
        }
        (ContentFilterDetection::BlockedWord(left), ContentFilterDetection::BlockedWord(right)) => {
            blocked_word_detections_equal(left, right)
        }
        (
            ContentFilterDetection::CategoryKeyword(left),
            ContentFilterDetection::CategoryKeyword(right),
        ) => category_keyword_equal(left, right),
        (
            ContentFilterDetection::CompetitorIntent(left),
            ContentFilterDetection::CompetitorIntent(right),
        ) => intent_detections_equal(left, right),
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
