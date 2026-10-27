#![cfg(test)]

use super::SseParser;

#[test]
fn parses_split_event() {
    let mut parser = SseParser::default();
    assert!(parser.push(b"data: {\"x\":").is_empty());
    assert_eq!(parser.push(b"1}\n\n"), vec![b"{\"x\":1}".to_vec()]);
}

#[test]
fn replays_the_sse_events_corpus() {
    let mut first = SseParser::default();
    let _events = first.push(&include_bytes!("../../fuzz/corpus/sse_events/basic")[..]);
    let _remainder = first.finish();
    let mut second = SseParser::default();
    let _second_events =
        second.push(&include_bytes!("../../fuzz/corpus/sse_events/split-boundary")[..]);
    let _second_remainder = second.finish();
}
