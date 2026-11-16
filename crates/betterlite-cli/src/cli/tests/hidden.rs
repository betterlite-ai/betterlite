use std::fs;
use std::path::Path;

use crate::cli::root::root_command;
use crate::cli::tests::help::listed_commands;
use crate::cli::tests::support::{FakeEnvironment, Harness};

fn environment_with_hidden(home: &Path, hidden: &str) {
    let directory = home.join(".litellm");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("config.json"),
        format!("{{\"hidden_commands\":\"{hidden}\"}}"),
    )
    .unwrap();
}

fn hidden_environment() -> (tempfile::TempDir, FakeEnvironment) {
    let home = tempfile::tempdir().unwrap();
    environment_with_hidden(home.path(), "models,config");
    let environment = FakeEnvironment::with_home(home.path().to_path_buf());
    (home, environment)
}

#[test]
fn hidden_commands_leave_the_help_listing() {
    let (_home, environment) = hidden_environment();
    let mut harness = Harness::new(environment);
    let code = harness.run(&["--help"]);
    assert_eq!(code, 0);
    let names = listed_commands(&harness.stdout());
    assert!(!names.iter().any(|name| name == "models"));
    assert!(!names.iter().any(|name| name == "config"));
    assert!(names.iter().any(|name| name == "credentials"));
}

#[test]
fn hidden_commands_still_parse() {
    let (_home, environment) = hidden_environment();
    let root = root_command(&environment);
    let matches = root
        .try_get_matches_from(["lite", "models", "list"])
        .expect("a hidden command still parses");
    let (name, _sub) = matches.subcommand().expect("the subcommand is present");
    assert_eq!(name, "models");
}

#[test]
fn hidden_commands_still_dispatch() {
    let (_home, environment) = hidden_environment();
    let mut harness = Harness::new(environment);
    let code = harness.run(&["models", "get"]);
    assert_eq!(code, 2);
    assert!(harness.stderr().contains("Either --id or --name"));
    assert!(!harness.stderr().contains("unrecognized subcommand"));
}

#[test]
fn the_statusline_command_is_hidden_but_parses() {
    let environment = FakeEnvironment::default();
    let root = root_command(&environment);
    let matches = root
        .try_get_matches_from(["lite", "statusline"])
        .expect("statusline parses");
    let (name, _sub) = matches.subcommand().expect("the subcommand is present");
    assert_eq!(name, "statusline");
}
