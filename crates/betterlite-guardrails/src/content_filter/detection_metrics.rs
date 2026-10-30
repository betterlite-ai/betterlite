use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::content_filter_records::ContentFilterDetection;

mod tests;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MatchDetail {
    pub r#type: String,
    pub action_taken: String,
    pub detection_method: String,
    pub snippet: String,
    pub category: Option<String>,
    pub confidence: Option<f64>,
}

pub fn count_masked_entities(
    detections: &[ContentFilterDetection],
    masked_entity_count: &mut BTreeMap<String, usize>,
) {
    for detection in detections {
        match detection {
            ContentFilterDetection::Pattern(pattern) if pattern.action == "MASK" => {
                increment(masked_entity_count, &pattern.pattern_name);
            }
            ContentFilterDetection::BlockedWord(word) if word.action == "MASK" => {
                increment(masked_entity_count, "blocked_word");
            }
            ContentFilterDetection::CategoryKeyword(category) if category.action == "MASK" => {
                increment(masked_entity_count, &category.category);
            }
            ContentFilterDetection::Pattern(_)
            | ContentFilterDetection::BlockedWord(_)
            | ContentFilterDetection::CategoryKeyword(_)
            | ContentFilterDetection::CompetitorIntent(_) => {}
        }
    }
}

#[must_use]
pub fn build_match_details(detections: &[ContentFilterDetection]) -> Vec<MatchDetail> {
    let mut details = Vec::with_capacity(detections.len());
    for detection in detections {
        details.push(match_detail(detection));
    }
    details
}

#[must_use]
pub fn get_detection_methods(detections: &[ContentFilterDetection]) -> String {
    let methods: BTreeSet<&str> = detections
        .iter()
        .map(|detection| match detection {
            ContentFilterDetection::Pattern(_) => "regex",
            ContentFilterDetection::CompetitorIntent(_) => "intent",
            ContentFilterDetection::BlockedWord(_) | ContentFilterDetection::CategoryKeyword(_) => {
                "keyword"
            }
        })
        .collect();
    methods.into_iter().collect::<Vec<_>>().join(",")
}

#[must_use]
pub const fn get_patterns_checked_count(
    compiled_patterns: usize,
    blocked_words: usize,
    category_keywords: usize,
    always_block_category_keywords: usize,
) -> usize {
    let mut total = compiled_patterns;
    total = total.saturating_add(blocked_words);
    total = total.saturating_add(category_keywords);
    total = total.saturating_add(always_block_category_keywords);
    total
}

#[must_use]
pub fn get_policy_templates(loaded_categories: &[(Option<String>, String)]) -> Option<String> {
    if loaded_categories.is_empty() {
        return None;
    }
    let names = loaded_categories
        .iter()
        .map(|(description, category_name)| description.as_deref().unwrap_or(category_name))
        .collect::<Vec<_>>();
    Some(names.join(", "))
}

fn increment(counts: &mut BTreeMap<String, usize>, name: &str) {
    let count = counts.entry(name.to_owned()).or_default();
    *count = count.saturating_add(1);
}

fn match_detail(detection: &ContentFilterDetection) -> MatchDetail {
    match detection {
        ContentFilterDetection::Pattern(pattern) => MatchDetail {
            r#type: pattern.r#type.clone(),
            action_taken: pattern.action.clone(),
            detection_method: "regex".to_owned(),
            snippet: pattern.pattern_name.clone(),
            category: None,
            confidence: None,
        },
        ContentFilterDetection::BlockedWord(word) => MatchDetail {
            r#type: word.r#type.clone(),
            action_taken: word.action.clone(),
            detection_method: "keyword".to_owned(),
            snippet: word.keyword.clone(),
            category: None,
            confidence: None,
        },
        ContentFilterDetection::CategoryKeyword(category) => MatchDetail {
            r#type: category.r#type.clone(),
            action_taken: category.action.clone(),
            detection_method: "keyword".to_owned(),
            snippet: category.keyword.clone(),
            category: Some(category.category.clone()),
            confidence: None,
        },
        ContentFilterDetection::CompetitorIntent(intent) => MatchDetail {
            r#type: intent.r#type.clone(),
            action_taken: intent.action_hint.clone(),
            detection_method: "intent".to_owned(),
            snippet: intent.intent.clone(),
            category: None,
            confidence: Some(intent.confidence),
        },
    }
}
