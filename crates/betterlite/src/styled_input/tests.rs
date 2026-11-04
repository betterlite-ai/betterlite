#![cfg(test)]

use std::io::Cursor;

use rstest::rstest;

use super::styled_input;

#[rstest]
#[case::tty_wraps_ansi_in_readline_markers(
    true,
    "\x1b[1mPick a provider\x1b[0m: ",
    "\u{1}\x1b[1m\u{2}Pick a provider\u{1}\x1b[0m\u{2}: "
)]
#[case::non_tty_strips_ansi_entirely(false, "\x1b[1mPick a provider\x1b[0m: ", "Pick a provider: ")]
#[case::plain_prompt_is_unchanged_either_way(true, "Provider(s): ", "Provider(s): ")]
fn renders_the_prompt_for_the_terminal_before_reading(
    #[case] is_tty: bool,
    #[case] prompt: &str,
    #[case] expected_written: &str,
) {
    let mut reader = Cursor::new(b"anthropic\n".to_vec());
    let mut writer = Vec::new();

    let line = styled_input(prompt, is_tty, &mut reader, &mut writer).expect("input succeeds");

    assert_eq!(line, "anthropic");
    assert_eq!(
        String::from_utf8(writer).expect("writer output is valid utf8"),
        expected_written
    );
}

#[test]
fn trims_surrounding_whitespace_from_the_entered_line() {
    let mut reader = Cursor::new(b"  sk-abc123  \n".to_vec());
    let mut writer = Vec::new();

    let line = styled_input("Key: ", false, &mut reader, &mut writer).expect("input succeeds");

    assert_eq!(line, "sk-abc123");
}

#[test]
fn eof_is_reported_to_the_caller() {
    let mut reader = Cursor::new(Vec::new());
    let mut writer = Vec::new();
    let error =
        styled_input("Key: ", false, &mut reader, &mut writer).expect_err("EOF is reported");
    assert!(
        matches!(error, super::StyledInputError::Io(ref source) if source.kind() == std::io::ErrorKind::UnexpectedEof)
    );
}
