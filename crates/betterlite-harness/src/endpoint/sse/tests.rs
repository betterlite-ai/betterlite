#![cfg(test)]

use super::{MAX_PENDING_LINE, SSEUsageParser};

const PREFIX: &str = "data: {\"usage\":{\"input_tokens\":5,\"output_tokens\":6},\"pad\":\"";
const SUFFIX: &str = "\"}";
const SMALL_LINE: &[u8] = b"data: {\"usage\":{\"input_tokens\":3,\"output_tokens\":4}}\n";

fn line_of(total: usize) -> Vec<u8> {
    let padding = total
        .saturating_sub(PREFIX.len())
        .saturating_sub(SUFFIX.len());
    format!("{PREFIX}{}{SUFFIX}", "x".repeat(padding)).into_bytes()
}

fn feed_parts(parts: &[&[u8]]) -> SSEUsageParser {
    let mut parser = SSEUsageParser::default();
    parts.iter().copied().for_each(|part| parser.feed(part));
    parser
}

fn feed_split(bytes: &[u8], at: usize) -> SSEUsageParser {
    let mut parser = SSEUsageParser::default();
    let (head, tail) = bytes.split_at(at);
    parser.feed(head);
    parser.feed(tail);
    parser
}

#[test]
fn a_stream_without_a_newline_stays_under_the_cap_and_later_lines_parse() {
    let chunk = vec![b'x'; 1 << 20];
    let mut parser = SSEUsageParser::default();
    (0_u8..40_u8).for_each(|_| {
        parser.feed(&chunk);
        assert!(parser.buffer.len() <= MAX_PENDING_LINE);
    });
    assert!(parser.buffer.is_empty());
    parser.feed(b"data: {\"usage\":{\"input_tokens\":7,\"output_tokens\":7}}\n");
    assert_eq!((parser.input_tokens, parser.output_tokens), (0, 0));
    parser.feed(SMALL_LINE);
    assert_eq!((parser.input_tokens, parser.output_tokens), (3, 4));
}

#[test]
fn a_line_of_exactly_the_cap_gives_its_usage_in_one_feed() {
    let mut line = line_of(MAX_PENDING_LINE);
    line.push(b'\n');
    let parser = feed_parts(&[line.as_slice()]);
    assert_eq!((parser.input_tokens, parser.output_tokens), (5, 6));
}

#[test]
fn a_line_of_exactly_the_cap_gives_its_usage_when_the_newline_comes_later() {
    let line = line_of(MAX_PENDING_LINE);
    let parser = feed_parts(&[line.as_slice(), b"\n"]);
    assert_eq!((parser.input_tokens, parser.output_tokens), (5, 6));
    assert!(parser.buffer.is_empty());
}

#[test]
fn a_line_of_exactly_the_cap_gives_its_usage_when_it_is_split_before_the_end() {
    let mut line = line_of(MAX_PENDING_LINE);
    line.push(b'\n');
    let parser = feed_split(&line, MAX_PENDING_LINE.saturating_sub(10));
    assert_eq!((parser.input_tokens, parser.output_tokens), (5, 6));
}

#[test]
fn a_line_over_the_cap_is_dropped_in_one_feed() {
    let mut line = line_of(MAX_PENDING_LINE.saturating_add(1));
    line.push(b'\n');
    let mut parser = feed_parts(&[line.as_slice()]);
    assert_eq!((parser.input_tokens, parser.output_tokens), (0, 0));
    parser.feed(SMALL_LINE);
    assert_eq!((parser.input_tokens, parser.output_tokens), (3, 4));
}

#[test]
fn a_line_over_the_cap_is_dropped_when_its_end_joins_a_pending_part() {
    let mut line = line_of(MAX_PENDING_LINE.saturating_add(1));
    line.push(b'\n');
    let mut parser = feed_split(&line, MAX_PENDING_LINE.saturating_sub(10));
    assert_eq!((parser.input_tokens, parser.output_tokens), (0, 0));
    parser.feed(SMALL_LINE);
    assert_eq!((parser.input_tokens, parser.output_tokens), (3, 4));
}

#[test]
fn a_line_over_the_cap_is_dropped_when_the_newline_comes_later() {
    let line = line_of(MAX_PENDING_LINE.saturating_add(1));
    let mut parser = feed_parts(&[line.as_slice()]);
    assert!(parser.buffer.is_empty());
    parser.feed(b"\n");
    assert_eq!((parser.input_tokens, parser.output_tokens), (0, 0));
    parser.feed(SMALL_LINE);
    assert_eq!((parser.input_tokens, parser.output_tokens), (3, 4));
}

#[test]
fn close_after_a_dropped_line_reads_nothing() {
    let line = line_of(MAX_PENDING_LINE.saturating_add(1));
    let mut parser = feed_parts(&[line.as_slice()]);
    parser.close();
    assert_eq!((parser.input_tokens, parser.output_tokens), (0, 0));
}
