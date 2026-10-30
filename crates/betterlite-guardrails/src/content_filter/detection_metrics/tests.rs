#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::{
    CategoryKeywordDetection, ContentFilterDetection, PatternDetection,
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
}
