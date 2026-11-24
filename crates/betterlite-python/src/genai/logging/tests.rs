#![cfg(test)]

use super::next_call_id;

#[test]
fn call_ids_are_distinct_and_prefixed() {
    let first = next_call_id();
    let second = next_call_id();
    assert!(first.starts_with("betterlite-genai-"));
    assert_ne!(first, second);
}
