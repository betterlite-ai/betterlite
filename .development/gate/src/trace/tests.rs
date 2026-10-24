#![cfg(test)]

use crate::trace::{pinned_range, split_line};

#[test]
fn a_pinned_link_gives_its_path_and_lines() {
    let range = pinned_range(
        "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L877-L892",
    )
    .unwrap();
    assert_eq!(range.path, "litellm/llms/azure/azure.py");
    assert_eq!(range.lines, Some((877, 892)));
}

#[test]
fn a_single_line_link_is_a_one_line_range() {
    let range =
        pinned_range("https://github.com/BerriAI/litellm/blob/abc/litellm/main.py#L6002").unwrap();
    assert_eq!(range.lines, Some((6002, 6002)));
}

#[test]
fn a_link_without_lines_names_the_whole_file() {
    let range =
        pinned_range("https://github.com/BerriAI/litellm/blob/abc/litellm/main.py").unwrap();
    assert_eq!(range.path, "litellm/main.py");
    assert_eq!(range.lines, None);
}

#[test]
fn a_docs_link_is_not_a_pinned_range() {
    assert!(pinned_range("https://docs.rs/axum/latest/axum/").is_none());
}

#[test]
fn a_trailing_number_is_the_line() {
    assert_eq!(
        split_line("litellm/main.py:6002"),
        ("litellm/main.py", Some(6002))
    );
    assert_eq!(
        split_line("crates/betterlite/src/main.rs"),
        ("crates/betterlite/src/main.rs", None)
    );
}
