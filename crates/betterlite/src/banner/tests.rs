#![cfg(test)]

use std::io::{self, Write};

use super::{BannerError, LITELLM_BANNER, show_banner};

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn show_banner_writes_the_banner_between_blank_lines() {
    let mut writer = Vec::new();

    show_banner(&mut writer).expect("writing succeeds");

    let output = String::from_utf8(writer).expect("writer output is valid utf8");
    assert_eq!(output, format!("\n{LITELLM_BANNER}\n\n"));
}

#[test]
fn banner_holds_six_block_letter_lines() {
    let lines: Vec<&str> = LITELLM_BANNER.split('\n').collect();

    assert_eq!(lines.len(), 6);
    assert!(lines.iter().all(|line| line.starts_with("   ")));
    assert_eq!(LITELLM_BANNER.lines().count(), 6);
}

#[test]
fn banner_starts_with_l_and_ends_with_the_last_letter() {
    let first = LITELLM_BANNER.lines().next().expect("six lines exist");
    let last = LITELLM_BANNER.lines().last().expect("six lines exist");

    assert!(first.trim_start().starts_with('\u{2588}'));
    assert!(first.ends_with('\u{2557}'));
    assert!(last.trim_start().starts_with('\u{255A}'));
    assert!(last.ends_with('\u{255D}'));
}

#[test]
fn show_banner_reports_a_writer_failure() {
    let error = show_banner(&mut FailingWriter).expect_err("the writer fails");

    assert!(matches!(error, BannerError::Io(_)));
}
