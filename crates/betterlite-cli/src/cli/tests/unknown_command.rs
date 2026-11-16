use crate::cli::tests::support::{FakeEnvironment, Harness};

#[test]
fn unknown_command_fails_with_a_usage_message() {
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["does-not-exist"]);
    assert_eq!(code, 2);
    assert!(harness.stderr().contains("unrecognized subcommand"));
    assert!(harness.stderr().contains("does-not-exist"));
}

#[test]
fn unknown_command_does_not_start_the_interactive_shell() {
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["does-not-exist"]);
    assert_eq!(code, 2);
    assert!(harness.stdout().is_empty());
}

#[test]
fn unknown_nested_command_fails_instead_of_reaching_the_command() {
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["models", "does-not-exist"]);
    assert_eq!(code, 2);
    assert!(harness.stderr().contains("unrecognized subcommand"));
}
