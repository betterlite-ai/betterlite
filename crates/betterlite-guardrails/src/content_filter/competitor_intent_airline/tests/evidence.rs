#![cfg(test)]

use super::shared::airline_checker;

#[test]
fn competitor_match_produces_entity_and_comparison_signal_evidence() {
    let result = airline_checker().run("Qatar Airways is better");
    let competitor = result
        .evidence
        .iter()
        .find(|entry| entry.key.as_deref() == Some("competitor"))
        .unwrap();
    assert_eq!(competitor.r#type.as_deref(), Some("entity"));
    assert_eq!(competitor.value.as_deref(), Some("qatar"));
    assert!(
        result
            .evidence
            .iter()
            .any(|entry| entry.key.as_deref() == Some("comparison"))
    );
}

#[test]
fn other_meaning_match_has_no_resolved_value() {
    let result = airline_checker().run("flight to Qatar next week");
    let other_meaning = result
        .evidence
        .iter()
        .find(|entry| entry.key.as_deref() == Some("other_meaning"))
        .unwrap();
    assert_eq!(other_meaning.r#type.as_deref(), Some("signal"));
    assert_eq!(other_meaning.value, None);
    assert_eq!(other_meaning.r#match.as_deref(), Some("qatar"));
}

#[test]
fn category_ranking_evidence_names_comparison_and_domain() {
    let result = airline_checker().run("best airline");
    let category = result
        .evidence
        .iter()
        .find(|entry| entry.key.as_deref() == Some("category_ranking"))
        .unwrap();
    assert_eq!(category.r#match.as_deref(), Some("comparison + domain"));
}
