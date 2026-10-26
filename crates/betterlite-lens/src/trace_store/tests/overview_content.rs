#![cfg(test)]

use super::{TracePart, overview_content};

fn part(parent: &str) -> TracePart {
    TracePart {
        execution_id: String::from("e"),
        span_id: String::from("s"),
        parent_span_id: String::from(parent),
        name: String::from("n"),
        kind: String::from("k"),
        content: "x".repeat(2_500),
        truncated: false,
    }
}

#[test]
fn formula_root_only() {
    assert!(overview_content(&part(""), 1).chars().count() < 2_500);
    assert!(overview_content(&part("r"), 1).chars().count() < 2_500);
}
