#![cfg(test)]
use super::{ListeningPortError, listening_port, port_in_line};
use rstest::rstest;
use std::io::{self, BufRead, Cursor, Read as _};

const LISTENING: &str = r#"{"timestamp":"t","level":"INFO","host":"127.0.0.1","port":40123,"message":"betterlite server listening"}"#;

#[rstest]
#[case::listening_line(LISTENING, Some(40123_u16))]
#[case::lowest_port(r#"{"port":0,"message":"betterlite server listening"}"#, Some(0_u16))]
#[case::highest_port(
    r#"{"port":65535,"message":"betterlite server listening"}"#,
    Some(65535_u16)
)]
#[case::port_past_the_range(r#"{"port":65536,"message":"betterlite server listening"}"#, None)]
#[case::other_message_with_a_port(
    r#"{"port":7,"message":"betterlite server shutting down"}"#,
    None
)]
#[case::listening_without_a_port(r#"{"message":"betterlite server listening"}"#, None)]
#[case::not_json("betterlite server listening", None)]
fn port_in_line_reads_only_the_listening_line(#[case] line: &str, #[case] expected: Option<u16>) {
    assert_eq!(port_in_line(line), expected);
}

#[test]
fn listening_port_skips_lines_before_the_listening_one() {
    let text = format!(
        "not json\n{}\n{LISTENING}\n",
        r#"{"port":7,"message":"other"}"#
    );
    let mut output = Cursor::new(text);
    assert_eq!(listening_port(&mut output).unwrap(), 40123_u16);
}

#[test]
fn listening_port_leaves_later_lines_for_the_caller() {
    let mut output = Cursor::new(format!("{LISTENING}\nlater\n"));
    listening_port(&mut output).unwrap();
    let mut rest = String::new();
    output.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "later\n");
}

#[test]
fn listening_port_ends_when_the_output_does() {
    let mut output = Cursor::new("not json\n");
    assert!(matches!(
        listening_port(&mut output),
        Err(ListeningPortError::Ended)
    ));
}

struct Broken;

impl io::Read for Broken {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("broken pipe"))
    }
}

impl BufRead for Broken {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::Error::other("broken pipe"))
    }

    fn consume(&mut self, _amount: usize) {}
}

#[test]
fn listening_port_reports_a_read_failure() {
    let error = listening_port(&mut Broken).unwrap_err();
    assert!(matches!(error, ListeningPortError::Read { .. }), "{error}");
}
