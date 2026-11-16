use crate::cli::tests::support::{FakeEnvironment, Harness};
use crate::cli::{Failure, failure_line};
use crate::commands::keys::KeysError;

#[test]
fn usage_shaped_command_errors_exit_two_with_one_stderr_line() {
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["models", "get"]);
    assert_eq!(code, 2);
    let stderr = harness.stderr();
    assert_eq!(stderr.lines().count(), 1);
    assert!(stderr.starts_with("Error: "));
    assert!(stderr.contains("Either --id or --name must be provided"));
}

#[test]
fn plain_command_errors_exit_one_with_one_stderr_line() {
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["whoami"]);
    assert_eq!(code, 1);
    let stderr = harness.stderr();
    assert_eq!(stderr.lines().count(), 1);
    assert_eq!(stderr, "Error: cannot find your home directory\n");
}

#[test]
fn usage_errors_exit_two_and_plain_errors_exit_one() {
    assert_eq!(Failure::Usage(String::from("bad")).code(), 2);
    assert_eq!(Failure::Plain(String::from("bad")).code(), 1);
}

#[test]
fn an_aborted_command_exits_one_and_says_aborted() {
    let failure = Failure::from(KeysError::Aborted);
    assert!(matches!(failure, Failure::Aborted(_)));
    assert_eq!(failure.code(), 1);
    assert_eq!(failure_line(&failure), "Aborted!");
}

#[test]
fn failures_print_one_error_line() {
    assert_eq!(
        failure_line(&Failure::Plain(String::from("no home"))),
        "Error: no home"
    );
    assert_eq!(
        failure_line(&Failure::Usage(String::from("bad flag"))),
        "Error: bad flag"
    );
}
