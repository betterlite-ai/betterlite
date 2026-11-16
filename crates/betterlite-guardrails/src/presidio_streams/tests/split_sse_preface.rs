#![cfg(test)]

use rstest::rstest;

use crate::presidio_streams::coalesce::split_sse_preface;

#[rstest]
#[case::comment_then_data(
    ": ka\n\nevent: x\n\ndata: {}\n\n",
    ": ka\n\nevent: x\n\n",
    "data: {}\n\n"
)]
#[case::data_first("data: a\n\n", "", "data: a\n\n")]
#[case::no_data_at_all(": ka\n\nevent: x\n\n", ": ka\n\nevent: x\n\n", "")]
#[case::empty("", "", "")]
#[case::crlf_frames(": ka\r\n\r\ndata: a\r\n\r\n", ": ka\r\n\r\n", "data: a\r\n\r\n")]
#[case::cr_frames(": ka\r\rdata: a\r\r", ": ka\r\r", "data: a\r\r")]
#[case::data_on_a_later_line("event: x\ndata: a\n\n", "", "event: x\ndata: a\n\n")]
#[case::data_after_cr_line("event: x\rdata: a\n\n", "", "event: x\rdata: a\n\n")]
#[case::data_word_inside_a_line(": data: a\n\nid: 1\n\n", ": data: a\n\nid: 1\n\n", "")]
#[case::data_needs_the_colon("data\n\ndata: a\n\n", "data\n\n", "data: a\n\n")]
fn split_sse_preface_cases(
    #[case] frames: &str,
    #[case] preface: &str,
    #[case] classifiable: &str,
) {
    let (before, rest) = split_sse_preface(frames.as_bytes());
    assert_eq!(str::from_utf8(before).unwrap(), preface);
    assert_eq!(str::from_utf8(rest).unwrap(), classifiable);
}
