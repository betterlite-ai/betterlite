use rstest::rstest;

use crate::agent_launch::KNOWN_AGENTS;
use crate::cli::root::root_command;
use crate::cli::tests::support::{FAMILY_COMMANDS, FakeEnvironment};

#[rstest]
fn every_family_command_is_mounted(#[values(FAMILY_COMMANDS)] command: &str) {
    let root = root_command(&FakeEnvironment::default());
    let names: Vec<&str> = root
        .get_subcommands()
        .map(|subcommand| subcommand.get_name())
        .collect();
    assert!(names.contains(&command), "{command} is not mounted");
}

#[test]
fn every_known_agent_is_mounted() {
    let root = root_command(&FakeEnvironment::default());
    let names: Vec<&str> = root
        .get_subcommands()
        .map(|subcommand| subcommand.get_name())
        .collect();
    let missing: Vec<&str> = KNOWN_AGENTS
        .iter()
        .map(|(binary, _display, _profiles, _hidden)| *binary)
        .filter(|binary| !names.contains(binary))
        .collect();
    assert!(missing.is_empty(), "{missing:?} are not mounted");
}

#[test]
fn statusline_is_mounted() {
    let root = root_command(&FakeEnvironment::default());
    let names: Vec<&str> = root
        .get_subcommands()
        .map(|subcommand| subcommand.get_name())
        .collect();
    assert!(names.contains(&"statusline"));
}
