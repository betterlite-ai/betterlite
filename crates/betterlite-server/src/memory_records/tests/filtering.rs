use crate::memory_records::filter::{KeyMatch, MemoryWhere, key_filter};

fn wire(filter: &MemoryWhere) -> String {
    serde_json::to_string(filter).unwrap()
}

#[test]
fn search_precedence_over_prefix_and_key() {
    let filter = key_filter(Some("mem"), Some("key-"), Some("exact")).unwrap();
    assert_eq!(
        wire(&filter),
        r#"{"OR":[{"key":{"startsWith":"mem"}},{"memory_id":"mem"}]}"#
    );
}

#[test]
fn search_matches_key_prefix() {
    let filter = key_filter(Some("team/notes"), None, None).unwrap();
    let MemoryWhere::AnyOf { any_of } = filter else {
        panic!("a search builds an OR of a key prefix and a memory id");
    };
    assert_eq!(any_of.len(), 2);
    assert_eq!(
        any_of.first(),
        Some(&MemoryWhere::Key {
            key: KeyMatch::StartsWith {
                starts_with: "team/notes".to_owned(),
            },
        })
    );
}

#[test]
fn search_matches_exact_memory_id() {
    let filter = key_filter(Some("9f1c2d3e-0000-4000-8000-000000000000"), None, None).unwrap();
    let MemoryWhere::AnyOf { any_of } = filter else {
        panic!("a search builds an OR of a key prefix and a memory id");
    };
    assert_eq!(
        any_of.get(1),
        Some(&MemoryWhere::MemoryId {
            memory_id: "9f1c2d3e-0000-4000-8000-000000000000".to_owned(),
        })
    );
}

#[test]
fn key_prefix_startswith() {
    let filter = key_filter(None, Some("team/"), None).unwrap();
    assert_eq!(wire(&filter), r#"{"key":{"startsWith":"team/"}}"#);
}

#[test]
fn key_exact_match() {
    let filter = key_filter(None, None, Some("team/notes")).unwrap();
    assert_eq!(wire(&filter), r#"{"key":"team/notes"}"#);
}

#[test]
fn empty_string_filter_remains() {
    let searched = key_filter(Some(""), Some("ignored"), Some("ignored")).unwrap();
    assert_eq!(
        wire(&searched),
        r#"{"OR":[{"key":{"startsWith":""}},{"memory_id":""}]}"#
    );
    let prefixed = key_filter(None, Some(""), Some("ignored")).unwrap();
    assert_eq!(wire(&prefixed), r#"{"key":{"startsWith":""}}"#);
    let exact = key_filter(None, None, Some("")).unwrap();
    assert_eq!(wire(&exact), r#"{"key":""}"#);
}

#[test]
fn none_filter_returns_none() {
    assert_eq!(key_filter(None, None, None), None);
}
