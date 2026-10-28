#![cfg(test)]

use betterlite_config::sections::content_filter_records::CodeBlockActionTaken;
use regex::Regex;
use rstest::rstest;

use super::{BlockDetector, CodeBlock, FENCED_BLOCK_PATTERN};

fn detector(blocked: &[&str], confidence_threshold: f64) -> BlockDetector {
    BlockDetector {
        fenced_block: Regex::new(FENCED_BLOCK_PATTERN).unwrap(),
        blocked_languages: blocked
            .iter()
            .map(|language| (*language).to_owned())
            .collect(),
        confidence_threshold,
    }
}

#[test]
fn finds_blocks_in_order_with_positions_tags_and_bodies() {
    let text = "intro\n```python\nprint(1)\nprint(2)\n```\nmid\n```\nls -la\n```";
    let blocks = detector(&["python"], 0.5).find_blocks(text);
    assert_eq!(
        blocks,
        vec![
            CodeBlock {
                start: 6,
                end: 37,
                language: "python".to_owned(),
                body: "print(1)\nprint(2)\n".to_owned(),
                confidence: 1.0,
                action_taken: CodeBlockActionTaken::Block,
            },
            CodeBlock {
                start: 42,
                end: 56,
                language: "(none)".to_owned(),
                body: "ls -la\n".to_owned(),
                confidence: 0.0,
                action_taken: CodeBlockActionTaken::Allow,
            },
        ]
    );
}

#[test]
fn positions_count_characters_not_bytes() {
    let text = "h\u{e9}llo \u{1f642} ```js\nx\n``` \u{e9}";
    let blocks = detector(&[], 0.5).find_blocks(text);
    let positions: Vec<(usize, usize)> = blocks
        .iter()
        .map(|block| (block.start, block.end))
        .collect();
    assert_eq!(positions, vec![(8, 19)]);
}

#[test]
fn keeps_the_tag_as_written() {
    let blocks = detector(&["javascript"], 0.5).find_blocks("```JS\nx\n```");
    let tags: Vec<&str> = blocks.iter().map(|block| block.language.as_str()).collect();
    assert_eq!(tags, vec!["JS"]);
}

#[rstest]
#[case::no_closing_fence("```python\nprint(1)\n")]
#[case::no_newline_after_tag("```python print(1)```")]
#[case::tag_with_dash("```objective-c\nx\n```")]
#[case::plain_text("no code here")]
fn ignores_text_without_a_fenced_block(#[case] text: &str) {
    assert_eq!(detector(&[], 0.5).find_blocks(text), Vec::new());
}

#[rstest]
#[case::listed_language(&["python"], 0.5_f64, "python", 1.0_f64, CodeBlockActionTaken::Block)]
#[case::alias_listed(&["python"], 1.0_f64, "py", 1.0_f64, CodeBlockActionTaken::Block)]
#[case::unlisted_language(&["python"], 0.5_f64, "rust", 0.0_f64, CodeBlockActionTaken::Allow)]
#[case::unlisted_at_zero_threshold(&["python"], 0.0_f64, "rust", 0.0_f64, CodeBlockActionTaken::Allow)]
#[case::block_all_executable(&[], 0.5_f64, "bash", 1.0_f64, CodeBlockActionTaken::Block)]
#[case::block_all_text_at_threshold(&[], 0.5_f64, "text", 0.5_f64, CodeBlockActionTaken::Block)]
#[case::block_all_text_below_threshold(&[], 0.8_f64, "text", 0.5_f64, CodeBlockActionTaken::LogOnly)]
fn assigns_confidence_and_action(
    #[case] blocked: &[&str],
    #[case] confidence_threshold: f64,
    #[case] tag: &str,
    #[case] confidence: f64,
    #[case] action_taken: CodeBlockActionTaken,
) {
    let text = format!("```{tag}\ncode\n```");
    let blocks = detector(blocked, confidence_threshold).find_blocks(&text);
    let scored: Vec<(u64, CodeBlockActionTaken)> = blocks
        .iter()
        .map(|block| (block.confidence.to_bits(), block.action_taken))
        .collect();
    assert_eq!(scored, vec![(confidence.to_bits(), action_taken)]);
}
