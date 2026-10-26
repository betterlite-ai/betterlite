#![cfg(test)]

use super::{TracePart, overview_content};
use rstest::rstest;

#[rstest]
#[case::zero_roots("", 0, 2000)]
#[case::one_root("", 1, 2000)]
#[case::six_roots("", 6, 2000)]
#[case::ten_roots("", 10, 1200)]
#[case::floor("", 100, 160)]
#[case::child("root", 1, 160)]
fn formula_root_only(#[case] parent: &str, #[case] roots: usize, #[case] limit: usize) {
    let mut part = TracePart {
        execution_id: String::from("e"),
        span_id: String::from("s"),
        parent_span_id: String::from(parent),
        name: String::from("n"),
        kind: String::from("k"),
        content: format!("{}{}", "\u{e9}".repeat(2500), "\u{5c3e}".repeat(2500)),
        truncated: false,
    };
    assert_eq!(
        overview_content(&part, roots),
        format!(
            "{}\n[... preview omitted; read this span for evidence ...]\n{}",
            "\u{e9}".repeat(limit.checked_div(3).unwrap()),
            "\u{5c3e}".repeat(limit.checked_mul(2).unwrap().checked_div(3).unwrap())
        )
    );
    part.content = "x".repeat(limit);
    assert_eq!(overview_content(&part, roots), part.content);
    part.content.clear();
    assert_eq!(overview_content(&part, roots), "");
}
