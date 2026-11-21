use betterlite_core::agents::json::AgentJson;
use rstest::rstest;

use super::split_lines;
use crate::agent_endpoints::a2a_stream::lines::{LineSplitter, event_from_line};

const NONE: [&str; 0] = [];

#[rstest]
#[case::line_feeds(b"a\nb\n", 4, &["a", "b"], &NONE, None)]
#[case::crlf(b"a\r\nb\r\n", 6, &["a", "b"], &NONE, None)]
#[case::crlf_across_chunks(b"a\r\nb\n", 2, &NONE, &["a", "b"], None)]
#[case::lone_cr(b"a\rb\rc", 4, &["a"], &["b"], Some("c"))]
#[case::two_crs(b"a\r\rb\n", 3, &["a"], &["", "b"], None)]
#[case::partial_line(b"data: 1", 2, &NONE, &NONE, Some("data: 1"))]
#[case::trailing_cr_at_the_end(b"a\r", 2, &NONE, &NONE, Some("a"))]
#[case::multibyte_across_chunks(b"\"\xc3\xa9\"\n", 2, &NONE, &["\"\u{e9}\""], None)]
#[case::invalid_utf8(b"a\xffb\xc3\n", 5, &["a\u{fffd}b\u{fffd}"], &NONE, None)]
#[case::empty_lines(b"\n\r\n\r", 4, &["", ""], &NONE, None)]
#[case::no_bytes(b"", 0, &NONE, &NONE, None)]
fn line_splitter_cuts_at_lf_crlf_and_cr(
    #[case] bytes: &[u8],
    #[case] split: usize,
    #[case] first: &[&str],
    #[case] second: &[&str],
    #[case] last: Option<&str>,
) {
    let (head, tail) = bytes.split_at(split);
    let mut splitter = LineSplitter::default();
    assert_eq!(splitter.push(head), first);
    assert_eq!(splitter.push(tail), second);
    assert_eq!(splitter.finish().as_deref(), last);
}

#[rstest]
#[case::split_inside_a_character(b"\tdata: \"\xc3\xa9\"\n", &["\tdata: \"\u{e9}\""])]
#[case::split_inside_crlf(b"\tdata: 1\r\ndata: 2\r\n", &["\tdata: 1", "data: 2"])]
#[case::split_past_the_end(b" data: 1", &[" data: 1"])]
fn split_input_gives_the_same_lines(#[case] bytes: &[u8], #[case] lines: &[&str]) {
    assert_eq!(split_lines(bytes), lines);
}

#[rstest]
#[case::object("data: {\"a\": 1}", Some("{\"a\": 1}"))]
#[case::no_space("data:7", Some("7"))]
#[case::padded("  data: [1] \t", Some("[1]"))]
#[case::unicode_spaces("\u{a0}data: 2\u{3000}", Some("2"))]
#[case::python_separators("\u{1c}data: 3\u{1f}", Some("3"))]
#[case::text("data: \"x\"", Some("\"x\""))]
#[case::null("data: null", Some("null"))]
#[case::empty("data:", None)]
#[case::blank("data:   ", None)]
#[case::invalid_json("data: {\"a\":", None)]
#[case::two_values("data: 1 2", None)]
#[case::event_line("event: message", None)]
#[case::comment(": data: 1", None)]
#[case::id_line("id: 1", None)]
#[case::retry_line("retry: 10", None)]
#[case::uppercase("Data: 1", None)]
#[case::other_prefix("xdata: 1", None)]
#[case::control_before("\u{0}data: 1", None)]
fn event_from_line_reads_only_data_lines(#[case] line: &str, #[case] event: Option<&str>) {
    let expected = event.map(|text| serde_json::from_str::<AgentJson>(text).unwrap());
    assert_eq!(event_from_line(line), expected);
}
