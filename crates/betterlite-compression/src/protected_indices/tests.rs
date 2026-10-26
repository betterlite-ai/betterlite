#![cfg(test)]

use super::*;
use serde_json::json;

#[test]
fn combines_and_deduplicates_protection_in_order() {
    let messages = vec![
        json!({"role":"system"}),
        json!({"role":"user"}),
        json!({"role":"assistant","cache_control":{"type":"ephemeral"}}),
        json!({"role":"user"}),
    ];
    assert_eq!(get_protected_indices(&messages), vec![0, 1, 2, 3]);
}

#[test]
fn cache_control_on_block_protects_prefix() {
    let messages = vec![
        json!({"role":"user","content":[{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}]}),
        json!({"role":"assistant"}),
    ];
    assert_eq!(cached_prefix_indices(&messages), vec![0]);
}
