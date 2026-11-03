#![cfg(test)]

use super::*;

fn chunk(wire: &str, text: &str, fragment_group_size: usize) -> ChunkResult {
    let item = format!(r#"{{"text":{{"text":"{text}"}}}}"#);
    ChunkResult {
        response: serde_json::from_str(wire).unwrap(),
        content: vec![serde_json::from_str(&item).unwrap()],
        fragment_group_size,
    }
}

fn merged_wire(chunks: &[ChunkResult]) -> String {
    serde_json::to_string(&merge(chunks).unwrap()).unwrap()
}

#[test]
fn split_refuses_empty_and_unsplittable_content() {
    assert!(matches!(split(&[]), Err(ChunkingError::EmptyContent)));
    assert!(split(&chunk("{}", "a", 1).content).unwrap().is_none());
    assert_eq!(nearest_whitespace("abcd "), 2);
    assert_eq!(nearest_whitespace("ab\u{1f}cdefgh"), 3);
    assert_eq!(bin_pack(&[], 4).len(), 1);
    let lowered = None::<&str>.map(str::to_lowercase).unwrap_or_default();
    assert!(!TOO_LARGE.iter().any(|pattern| lowered.contains(pattern)));
}

#[test]
fn merge_reassembles_fragments_and_keeps_late_nulls() {
    let chunks = [
        chunk(r#"{"outputs":[],"action":"NONE"}"#, "safe", 1),
        chunk(r#"{"outputs":[{"text":"A"}]}"#, "a", 2),
        chunk(r#"{"outputs":[{"text":null}],"action":null}"#, "b", 2),
    ];
    let outputs = r#"[{"text":"safe"},{"text":"Ab"}]"#;
    let expected = format!(r#"{{"action":"NONE","output":{outputs},"outputs":{outputs}}}"#);
    assert_eq!(merged_wire(&chunks), expected);
    let unmasked = [chunk(r#"{"outputs":[],"action":null}"#, "a", 1)];
    assert_eq!(merged_wire(&unmasked), r#"{"outputs":[],"action":null}"#);
}

#[test]
fn merge_refuses_a_usage_total_past_i64() {
    let huge = format!(r#"{{"usage":{{"u":{}}}}}"#, i64::MAX);
    let overflow = merge(&[chunk(&huge, "a", 1), chunk(&huge, "b", 1)]);
    assert!(matches!(overflow, Err(ChunkingError::UsageOverflow { counter }) if counter == "u"));
}

#[test]
fn bin_pack_preserves_content_and_splits_at_the_character_budget() {
    let content: Vec<_> = ["ab", "cd", "\u{e9}x"]
        .into_iter()
        .flat_map(|text| chunk("{}", text, 1).content)
        .collect();
    let batches = bin_pack(&content, 4);
    let texts: Vec<Vec<_>> = batches
        .iter()
        .map(|batch| {
            batch
                .iter()
                .map(|item| item.text.as_ref().unwrap().text.as_deref().unwrap())
                .collect()
        })
        .collect();
    assert_eq!(texts, vec![vec!["ab", "cd"], vec!["\u{e9}x"]]);
}
