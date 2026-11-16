#![cfg(test)]

use std::collections::VecDeque;
use std::io;

use rstest::rstest;

use crate::cli::Failure;
use crate::cli::tests::support::{Captured, FakeEnvironment, FakeVault, session};
use crate::commands_listing::show_commands;
use crate::shell::{LineReader, report_in_session, run};

struct Scripted(VecDeque<String>);

impl Scripted {
    fn new(lines: &[&str]) -> Self {
        Self(lines.iter().map(|line| (*line).to_owned()).collect())
    }
}

impl LineReader for Scripted {
    fn read_line(&mut self) -> io::Result<Option<String>> {
        Ok(self.0.pop_front())
    }
}

struct Run {
    result: Result<(), Failure>,
    stdout: String,
    stderr: String,
}

fn run_shell(lines: &[&str]) -> Run {
    let environment = FakeEnvironment::default();
    let mut vault = FakeVault;
    let out = Captured::default();
    let err = Captured::default();
    let mut session = session(&environment, &mut vault, "http://proxy.example", &out, &err);
    let mut reader = Scripted::new(lines);
    let result = run(&mut session, &mut reader);
    Run {
        result,
        stdout: out.text(),
        stderr: err.text(),
    }
}

#[test]
fn starts_with_the_banner_and_connection_line() {
    let outcome = run_shell(&["exit"]);
    outcome.result.unwrap();
    assert!(
        outcome
            .stdout
            .contains("Connected to betterlite server: http://proxy.example")
    );
    assert!(outcome.stdout.contains("Available commands:"));
    assert!(outcome.stderr.is_empty());
}

#[test]
fn the_listing_comes_from_the_shared_commands_listing() {
    let environment = FakeEnvironment::default();
    let mut expected = Vec::new();
    show_commands(&mut expected, &environment).unwrap();
    let expected = String::from_utf8(expected).unwrap();
    let outcome = run_shell(&["help", "exit"]);
    assert!(outcome.stdout.contains(&expected));
}

#[test]
fn help_prints_the_listing_again_and_keeps_the_session_open() {
    let outcome = run_shell(&["help", "exit"]);
    outcome.result.unwrap();
    assert_eq!(outcome.stdout.matches("Available commands:").count(), 2);
    assert!(outcome.stdout.trim_end().ends_with("Goodbye!"));
}

#[test]
fn clear_redraws_the_banner_and_listing() {
    let outcome = run_shell(&["clear", "exit"]);
    outcome.result.unwrap();
    assert_eq!(outcome.stdout.matches(super::BANNER).count(), 2);
    assert_eq!(outcome.stdout.matches("Available commands:").count(), 2);
}

#[test]
fn clearing_the_screen_only_writes_to_a_terminal() {
    let mut written = Vec::new();
    super::clear_screen(&mut written, false).unwrap();
    assert!(written.is_empty());
    super::clear_screen(&mut written, true).unwrap();
    assert_eq!(String::from_utf8(written).unwrap(), "\x1b[2J\x1b[1;1H");
}

#[rstest]
#[case::exit("exit")]
#[case::quit("quit")]
fn exit_and_quit_both_end_the_session(#[case] word: &str) {
    let outcome = run_shell(&[word]);
    outcome.result.unwrap();
    assert!(
        outcome.stdout.trim_end().ends_with("Goodbye!"),
        "{word} must say goodbye"
    );
}

#[test]
fn an_unknown_line_is_reported_and_the_session_continues() {
    let outcome = run_shell(&["nonsense", "exit"]);
    outcome.result.unwrap();
    assert!(outcome.stdout.contains("Unknown command: nonsense"));
    assert!(
        outcome
            .stdout
            .contains("Type 'help' to see available commands.")
    );
    assert!(outcome.stdout.trim_end().ends_with("Goodbye!"));
}

#[test]
fn an_empty_line_is_ignored() {
    let outcome = run_shell(&["", "exit"]);
    outcome.result.unwrap();
    assert!(!outcome.stdout.contains("Unknown command:"));
    assert!(outcome.stdout.trim_end().ends_with("Goodbye!"));
}

#[test]
fn a_line_is_redispatched_through_the_shared_clap_root() {
    let outcome = run_shell(&["models get", "exit"]);
    outcome.result.unwrap();
    assert!(outcome.stderr.contains("Error: Either --id or --name"));
    assert!(outcome.stdout.trim_end().ends_with("Goodbye!"));
}

#[test]
fn a_usage_error_in_the_shell_is_printed_without_ending_the_loop() {
    let outcome = run_shell(&["models get --bogus", "exit"]);
    outcome.result.unwrap();
    assert!(outcome.stderr.contains("unexpected argument"));
    assert!(outcome.stdout.trim_end().ends_with("Goodbye!"));
}

#[test]
fn end_of_input_says_goodbye() {
    let outcome = run_shell(&[]);
    outcome.result.unwrap();
    assert!(outcome.stdout.contains("\nGoodbye!"));
}

#[test]
fn the_prompt_draws_the_box_and_moves_the_cursor() {
    let outcome = run_shell(&["exit"]);
    outcome.result.unwrap();
    let rule = format!("+{}+", "-".repeat(78));
    assert!(outcome.stdout.contains(&rule));
    assert!(outcome.stdout.contains(&format!("| {} |", " ".repeat(76))));
    assert!(outcome.stdout.contains("\x1b[2A\r| > "));
    assert!(outcome.stdout.contains("\x1b[1B"));
}

#[test]
fn an_aborted_command_prints_command_aborted_on_stdout() {
    let environment = FakeEnvironment::default();
    let mut vault = FakeVault;
    let out = Captured::default();
    let err = Captured::default();
    let mut session = session(&environment, &mut vault, "http://proxy.example", &out, &err);
    report_in_session(&mut session, &Failure::Aborted(String::from("stopped"))).unwrap();
    assert_eq!(out.text(), "Command aborted.\n");
    assert!(err.text().is_empty());
}

#[test]
fn a_failed_command_prints_one_error_line_on_stderr() {
    let environment = FakeEnvironment::default();
    let mut vault = FakeVault;
    let out = Captured::default();
    let err = Captured::default();
    let mut session = session(&environment, &mut vault, "http://proxy.example", &out, &err);
    report_in_session(&mut session, &Failure::Plain(String::from("bad"))).unwrap();
    assert_eq!(err.text(), "Error: bad\n");
    assert!(out.text().is_empty());
}
