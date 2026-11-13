#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::{
    BlockedWordDetection, CategoryKeywordDetection, ContentFilterDetection, PatternDetection,
};

use super::{
    build_match_details, count_masked_entities, get_detection_methods, get_patterns_checked_count,
    get_policy_templates,
};

#[test]
fn counts_only_masked_entity_types() {
    let detections = vec![
        ContentFilterDetection::Pattern(PatternDetection {
            r#type: "pattern".to_owned(),
            pattern_name: "email".to_owned(),
            action: "MASK".to_owned(),
        }),
        ContentFilterDetection::Pattern(PatternDetection {
            r#type: "pattern".to_owned(),
            pattern_name: "secret".to_owned(),
            action: "BLOCK".to_owned(),
        }),
        ContentFilterDetection::CategoryKeyword(CategoryKeywordDetection {
            r#type: "category_keyword".to_owned(),
            category: "pii".to_owned(),
            keyword: "address".to_owned(),
            severity: "high".to_owned(),
            action: "MASK".to_owned(),
        }),
    ];
    let mut counts = BTreeMap::new();
    count_masked_entities(&detections, &mut counts);
    assert_eq!(counts.get("email"), Some(&1));
    assert_eq!(counts.get("pii"), Some(&1));
    assert!(!counts.contains_key("secret"));
}

#[rstest::rstest]
#[case::mask("MASK", 3, Some(1))]
#[case::block("BLOCK", 2, None)]
#[case::allow("ALLOW", 2, None)]
fn blocked_words_and_categories_count_only_mask_actions(
    #[case] action: &str,
    #[case] word_count: usize,
    #[case] category_count: Option<usize>,
) {
    let detections = vec![
        ContentFilterDetection::BlockedWord(BlockedWordDetection {
            r#type: "blocked_word".to_owned(),
            keyword: "secret".to_owned(),
            action: action.to_owned(),
            description: None,
        }),
        ContentFilterDetection::CategoryKeyword(CategoryKeywordDetection {
            r#type: "category_keyword".to_owned(),
            category: "pii".to_owned(),
            keyword: "address".to_owned(),
            severity: "high".to_owned(),
            action: action.to_owned(),
        }),
    ];
    let mut counts = BTreeMap::from([("blocked_word".to_owned(), 2)]);
    count_masked_entities(&detections, &mut counts);
    assert_eq!(counts.get("blocked_word"), Some(&word_count));
    assert_eq!(counts.get("pii"), category_count.as_ref());
    assert_eq!(counts.len(), category_count.map_or(1, |_| 2));
}

#[test]
fn formats_details_and_methods() {
    let detections = vec![ContentFilterDetection::Pattern(PatternDetection {
        r#type: "pattern".to_owned(),
        pattern_name: "email".to_owned(),
        action: "MASK".to_owned(),
    })];
    let details = build_match_details(&detections);
    let detail = details.first().expect("one detail");
    assert_eq!(detail.detection_method, "regex");
    assert_eq!(detail.snippet, "email");
    assert_eq!(get_detection_methods(&detections), "regex");
    assert_eq!(get_detection_methods(&[]), "");
}

#[test]
fn sums_patterns_and_templates() {
    assert_eq!(get_patterns_checked_count(1, 2, 3, 4), 10);
    assert_eq!(get_policy_templates(&[]), None);
    assert_eq!(
        get_policy_templates(&[(Some("strict".to_owned()), "pii".to_owned())]),
        Some("strict".to_owned())
    );
    assert_eq!(
        get_policy_templates(&[(Some(String::new()), "pii".to_owned())]),
        Some("pii".to_owned())
    );
}
