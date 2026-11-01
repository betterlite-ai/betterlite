#![cfg(test)]

use super::parse_frontmatter;
use rstest::rstest;

#[rstest]
fn splits_frontmatter_from_the_template_body() {
    let (metadata, content) =
        parse_frontmatter("---\nmodel: gpt-4\n---\nHello {{ name }}").unwrap();
    assert_eq!(metadata.get("model").unwrap().as_str(), Some("gpt-4"));
    assert_eq!(content, "Hello {{ name }}");
}

#[rstest]
fn content_without_a_delimiter_is_returned_unchanged() {
    let (metadata, content) = parse_frontmatter("Hello {{ name }}").unwrap();
    assert!(metadata.is_empty());
    assert_eq!(content, "Hello {{ name }}");
}

#[rstest]
fn an_empty_frontmatter_block_yields_empty_metadata() {
    let (metadata, content) = parse_frontmatter("---\n\n---\nbody").unwrap();
    assert!(metadata.is_empty());
    assert_eq!(content, "body");
}

#[rstest]
fn invalid_yaml_raises_instead_of_falling_back_to_content() {
    let result = parse_frontmatter("---\nmodel: [unterminated\n---\nbody");
    result.unwrap_err();
}

#[rstest]
fn a_non_mapping_frontmatter_is_rejected() {
    let result = parse_frontmatter("---\n- one\n- two\n---\nbody");
    result.unwrap_err();
}

#[rstest]
fn trailing_blank_lines_after_the_closing_delimiter_are_consumed_too() {
    // `\s*\n` is greedy: it swallows every blank line after the closing `---`, not just the first.
    let (_, content) = parse_frontmatter("---\nmodel: gpt-4\n---\n\nbody").unwrap();
    assert_eq!(content, "body");
}
