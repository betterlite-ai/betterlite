#![cfg(test)]

use rstest::rstest;

use super::{LineAccumulator, MAX_LINE_BYTES, MAX_TEXT_BYTES, SseError, accumulate_text};

fn lines_from(chunks: &[&[u8]]) -> Vec<String> {
    let mut accumulator = LineAccumulator::default();
    let mut lines = Vec::new();
    for chunk in chunks {
        lines.extend(accumulator.push(chunk).unwrap());
    }
    if let Some(line) = accumulator.finish() {
        lines.push(line);
    }
    lines
}

#[test]
fn holds_a_line_until_a_later_chunk_ends_it() {
    assert_eq!(
        lines_from(&[b"data: par", b"tial\n"]),
        ["data: partial".to_owned()]
    );
}

#[test]
fn splits_every_line_a_chunk_carries() {
    assert_eq!(
        lines_from(&[b"one\ntwo\nthree"]),
        ["one".to_owned(), "two".to_owned(), "three".to_owned()]
    );
}

#[test]
fn keeps_the_final_line_that_has_no_newline() {
    assert_eq!(
        lines_from(&[b"one\ntwo"]),
        ["one".to_owned(), "two".to_owned()]
    );
}

#[test]
fn joins_a_carriage_return_and_newline_split_across_chunks() {
    assert_eq!(
        lines_from(&[b"one\r", b"\ntwo\r\n"]),
        ["one".to_owned(), "two".to_owned()]
    );
}

#[test]
fn treats_a_bare_carriage_return_as_a_line_ending() {
    assert_eq!(
        lines_from(&[b"one\rtwo\n"]),
        ["one".to_owned(), "two".to_owned()]
    );
}

#[test]
fn a_carriage_return_at_the_end_of_the_input_ends_its_line() {
    assert_eq!(lines_from(&[b"one\r"]), ["one".to_owned()]);
}

#[test]
fn drops_a_line_that_is_not_text() {
    assert_eq!(lines_from(&[b"data: \xff\nnext\n"]), ["next".to_owned()]);
}

#[test]
fn a_line_at_the_bound_still_passes() {
    let line = vec![b'a'; MAX_LINE_BYTES];
    let mut accumulator = LineAccumulator::default();
    assert_eq!(accumulator.push(&line).unwrap().len(), 0);
    assert_eq!(accumulator.push(b"\n").unwrap()[0].len(), MAX_LINE_BYTES);
}

#[test]
fn a_line_past_the_bound_is_an_error() {
    let line = vec![b'a'; MAX_LINE_BYTES + 1];
    let mut accumulator = LineAccumulator::default();
    assert!(matches!(
        accumulator.push(&line),
        Err(SseError::LineTooLong {
            max: MAX_LINE_BYTES
        })
    ));
}

#[test]
fn reads_only_lines_that_start_with_a_data_prefix() {
    let lines = [
        "event: status".to_owned(),
        "data: {\"status\":\"completed\",\"last_message\":{\"content\":[{\"text\":\"kept\"}]}}"
            .to_owned(),
        "id: 7".to_owned(),
    ];
    assert_eq!(accumulate_text(&lines).unwrap(), "kept");
}

#[rstest]
#[case::no_space("data:{\"last_message\":{\"content\":[{\"text\":\"a\"}]}}")]
#[case::extra_space("data:   {\"last_message\":{\"content\":[{\"text\":\"a\"}]}}  ")]
fn strips_the_data_prefix_and_padding(#[case] line: &str) {
    assert_eq!(accumulate_text(&[line.to_owned()]).unwrap(), "a");
}

#[rstest]
#[case::empty("data: ")]
#[case::done("data: [DONE]")]
#[case::padded_done("data:  [DONE]  ")]
#[case::malformed("data: {\"status\": ")]
#[case::wrong_type("data: {\"status\": 5}")]
#[case::not_json("data: hello")]
fn skips_payloads_without_run_text(#[case] line: &str) {
    assert_eq!(accumulate_text(&[line.to_owned()]).unwrap(), "");
}

#[test]
fn joins_the_text_of_every_event() {
    let lines = [
        "data: {\"last_message\":{\"content\":[{\"text\":\"one \"}]}}".to_owned(),
        "data: {\"result\":{\"data\":{\"message\":{\"content\":[{\"text\":\"two\"}]}}}}".to_owned(),
        "data: {\"results\":\"three\"}".to_owned(),
    ];
    assert_eq!(accumulate_text(&lines).unwrap(), "one two three");
}

#[test]
fn an_event_without_text_adds_nothing() {
    let lines = ["data: {\"status\":\"completed\"}".to_owned()];
    assert_eq!(accumulate_text(&lines).unwrap(), "");
}

fn long_text_event(length: usize) -> String {
    let text = "a".repeat(length);
    format!("data: {{\"last_message\": {{\"content\": [{{\"text\": \"{text}\"}}]}}}}")
}

#[test]
fn text_at_the_bound_still_passes() {
    let line = long_text_event(MAX_TEXT_BYTES);
    assert_eq!(accumulate_text(&[line]).unwrap().len(), MAX_TEXT_BYTES);
}

#[test]
fn text_past_the_bound_is_an_error() {
    let line = long_text_event(MAX_TEXT_BYTES + 1);
    assert!(matches!(
        accumulate_text(&[line]),
        Err(SseError::TextTooLong {
            max: MAX_TEXT_BYTES
        })
    ));
}
