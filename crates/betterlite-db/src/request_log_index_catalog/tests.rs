#![cfg(test)]

use super::{REQUEST_LOG_INDEXES, RequestLogIndex};
use rstest::rstest;

#[test]
fn registry_order_and_columns() {
    assert_eq!(REQUEST_LOG_INDEXES[0].columns(), ["api_key", "startTime"]);
    assert_eq!(REQUEST_LOG_INDEXES[1].columns(), ["litellm_call_id"]);
}

#[test]
fn child_name_matching_prefix() {
    assert_eq!(
        REQUEST_LOG_INDEXES[0].partition_index_name("part"),
        "part_api_key_startTime_idx"
    );
}

#[test]
fn child_name_nonmatching_prefix() {
    let index = RequestLogIndex {
        table: "Other",
        name: "LiteLLM_SpendLogs_idx",
        definition: "(\"x\")",
    };
    assert_eq!(
        index.partition_index_name("part"),
        "part_LiteLLM_SpendLogs_idx"
    );
}

#[test]
fn name_exactly_sixty_three_bytes() {
    let index_name = format!("T_{}", "a".repeat(61));
    let index = RequestLogIndex {
        table: "T",
        name: &index_name,
        definition: "(\"x\")",
    };
    let name = index.partition_index_name("p");
    assert_eq!(name.len(), 63);
}

#[test]
fn long_name_full_digest() {
    let index = RequestLogIndex {
        table: "T",
        name: "T_a_very_long_index_name_that_needs_truncation",
        definition: "(\"x\")",
    };
    let name = index.partition_index_name(&"p".repeat(100));
    assert_eq!(name.len(), 63);
    assert!(name.ends_with("_904b19ee"));
}

#[test]
fn long_unicode_name_boundary() {
    let index = RequestLogIndex {
        table: "T",
        name: "T_suffix",
        definition: "(\"x\")",
    };
    let name = index.partition_index_name(&"界".repeat(30));
    assert!(name.is_char_boundary(54));
    assert_eq!(name.len(), 63);
}

#[rstest]
#[case::different_suffix("first", "second")]
#[case::same_prefix_different_suffix("a", "b")]
fn long_suffixes_remain_distinct(#[case] first: &str, #[case] second: &str) {
    let index = RequestLogIndex {
        table: "T",
        name: "T_suffix",
        definition: "(\"x\")",
    };
    assert_ne!(
        index.partition_index_name(&format!("{}{}", "p".repeat(100), first)),
        index.partition_index_name(&format!("{}{}", "p".repeat(100), second))
    );
}
