#![cfg(test)]
use rstest::rstest;

use crate::sandbox_backend::protocol::{Encoding, FileData, ReadResult, ReadWindow};
use crate::sandbox_backend::read_window::slice_read;

fn window(
    content: &str,
    total_lines: usize,
    start_line: usize,
    end_line: usize,
    next_offset: Option<usize>,
) -> ReadResult {
    ReadResult::Window {
        file_data: FileData {
            content: content.to_owned(),
            encoding: Encoding::Utf8,
        },
        window: ReadWindow {
            total_lines,
            start_line,
            end_line,
            next_offset,
        },
    }
}

#[rstest]
#[case::first_lines("a\nb\nc\n", 0, 2, window("a\nb\n", 3, 1, 2, Some(2)))]
#[case::last_lines("a\nb\nc", 1, 5, window("b\nc", 3, 2, 3, None))]
#[case::crlf_becomes_lf("a\r\nb\r\n", 0, 2000, window("a\nb\n", 2, 1, 2, None))]
#[case::lone_cr_breaks_a_line("a\rb", 1, 1, window("b", 2, 2, 2, None))]
#[case::form_feed_breaks_a_line("a\u{c}b", 0, 1, window("a\u{c}", 2, 1, 1, Some(1)))]
#[case::line_separator_breaks_a_line("a\u{2028}b", 1, 1, window("b", 2, 2, 2, None))]
fn slices_the_requested_lines(
    #[case] content: &str,
    #[case] offset: usize,
    #[case] limit: usize,
    #[case] expected: ReadResult,
) {
    assert_eq!(slice_read(content.to_owned(), offset, limit), expected);
}

#[rstest]
#[case::empty("")]
#[case::whitespace(" \n\t\n")]
fn returns_blank_content_whole(#[case] content: &str) {
    assert_eq!(
        slice_read(content.to_owned(), 3, 0),
        ReadResult::Whole(FileData {
            content: content.to_owned(),
            encoding: Encoding::Utf8,
        })
    );
}

#[test]
fn flags_a_zero_limit() {
    assert_eq!(
        slice_read("a\n".to_owned(), 0, 0),
        ReadResult::NoLinesRequested
    );
}

#[test]
fn refuses_an_offset_past_the_end() {
    assert_eq!(
        slice_read("a\nb\n".to_owned(), 2, 10),
        ReadResult::Error("Line offset 2 exceeds file length (2 lines)".to_owned())
    );
}
