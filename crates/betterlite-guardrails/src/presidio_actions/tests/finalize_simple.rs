#![cfg(test)]

use indexmap::IndexMap;

use crate::presidio_actions::anonymize::{AnonymizeResponse, finalize_simple};

#[test]
fn returns_the_anonymizer_text_and_counts_each_entity_type() {
    let redacted: AnonymizeResponse = serde_json::from_str(
        r#"{"text":"<PERSON> met <PERSON>","items":[{"entity_type":"PERSON"},{"entity_type":"PERSON"},{"entity_type":"EMAIL_ADDRESS"},{"entity_type":null},{}]}"#,
    )
    .unwrap();
    let mut counts = IndexMap::new();
    let text = finalize_simple(&redacted, &mut counts);
    assert_eq!(text, "<PERSON> met <PERSON>");
    assert_eq!(counts.get("PERSON"), Some(&2));
    assert_eq!(counts.get("EMAIL_ADDRESS"), Some(&1));
    assert_eq!(counts.len(), 2);
}

#[test]
fn missing_items_count_nothing_and_still_return_the_text() {
    let redacted: AnonymizeResponse = serde_json::from_str(r#"{"text":"clean"}"#).unwrap();
    let mut counts = IndexMap::new();
    assert_eq!(finalize_simple(&redacted, &mut counts), "clean");
    assert!(counts.is_empty());
}
