#![cfg(test)]

use std::error::Error as _;
use std::io;
use std::path::PathBuf;

use rstest::rstest;

use super::DebugError;
use crate::commands::ClientError;
use crate::settings_backup::error::BackupError;

fn path() -> PathBuf {
    PathBuf::from("/tmp/x")
}

fn io_error() -> io::Error {
    io::Error::from(io::ErrorKind::PermissionDenied)
}

fn bad_json() -> serde_json::Error {
    serde_json::from_str::<u8>("x").unwrap_err()
}

fn client_error() -> ClientError {
    ClientError::Url {
        url: "x".to_owned(),
        source: url::ParseError::EmptyHost,
    }
}

#[rstest]
#[case::no_home(DebugError::NoHome, false, "the environment has no home directory")]
#[case::no_session(
    DebugError::NoSession,
    false,
    "Could not find a Claude Code session. Pass --session-id or set $CLAUDE_CODE_SESSION_ID."
)]
#[case::transcript_scan(
    DebugError::TranscriptScan { path: path(), source: io_error() },
    true,
    "cannot list the Claude Code transcripts in /tmp/x"
)]
#[case::request(
    DebugError::Request { uri: "/spend/logs/ui/r1".to_owned(), source: client_error() },
    true,
    "GET /spend/logs/ui/r1 failed"
)]
#[case::status(
    DebugError::Status { uri: "/spend/logs/ui/r1".to_owned(), status: 502, body: "bad gateway".to_owned() },
    false,
    "GET /spend/logs/ui/r1 failed with HTTP 502: bad gateway"
)]
#[case::unexpected_shape(
    DebugError::UnexpectedShape { uri: "/spend/logs/ui/r1".to_owned(), source: bad_json() },
    true,
    "Unexpected /spend/logs/ui/r1 response"
)]
#[case::no_session_rows(
    DebugError::NoSessionRows { session_id: "s1".to_owned(), base_url: "http://proxy".to_owned() },
    false,
    "No spend logs found for session 's1' on http://proxy. Is Claude Code routed through this proxy (`lite up`), and does your key have log access?"
)]
#[case::spend_overflow(
    DebugError::SpendOverflow,
    false,
    "the total spend of the session does not fit a decimal"
)]
#[case::session_id_not_file_name(
    DebugError::SessionIdNotFileName { session_id: "a/b".to_owned() },
    false,
    "the session id 'a/b' cannot be part of a file name"
)]
#[case::report_directory(
    DebugError::ReportDirectory { path: path(), source: io_error() },
    true,
    "cannot create the directory /tmp/x"
)]
#[case::report_create(
    DebugError::ReportCreate {
        path: path(),
        source: BackupError::Create { path: path(), source: io_error() },
    },
    true,
    "cannot create the report file /tmp/x"
)]
#[case::report_write(
    DebugError::ReportWrite { path: path(), source: io_error() },
    true,
    "cannot write the report to /tmp/x"
)]
#[case::slash_command_directory(
    DebugError::SlashCommandDirectory { path: path(), source: io_error() },
    true,
    "cannot create the directory /tmp/x"
)]
#[case::slash_command_write(
    DebugError::SlashCommandWrite { path: path(), source: io_error() },
    true,
    "cannot write the slash command to /tmp/x"
)]
#[case::output(DebugError::from(io_error()), true, "cannot write to the terminal")]
fn debug_error_names_the_failure_and_carries_its_source(
    #[case] error: DebugError,
    #[case] has_source: bool,
    #[case] message: &str,
) {
    assert_eq!(error.to_string(), message);
    assert_eq!(error.source().is_some(), has_source);
}

#[test]
fn debug_error_keeps_the_io_error_of_a_terminal_write() {
    let error = DebugError::from(io_error());
    let source = error.source().unwrap();

    assert_eq!(source.to_string(), io_error().to_string());
}
