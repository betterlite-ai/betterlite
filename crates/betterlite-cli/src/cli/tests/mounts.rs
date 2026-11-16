use clap::Command;
use rstest::rstest;

use crate::agent_launch::KNOWN_AGENTS;
use crate::cli::root::root_command;
use crate::cli::tests::support::FakeEnvironment;

#[rstest]
#[case::auth("auth")]
#[case::autoroute("autoroute")]
#[case::chat("chat")]
#[case::config("config")]
#[case::configure("configure")]
#[case::credentials("credentials")]
#[case::debug("debug")]
#[case::down("down")]
#[case::encryption("encryption")]
#[case::http("http")]
#[case::keys("keys")]
#[case::login("login")]
#[case::logout("logout")]
#[case::model_groups("model-groups")]
#[case::models("models")]
#[case::reconfigure("reconfigure")]
#[case::teams("teams")]
#[case::unconfigure("unconfigure")]
#[case::up("up")]
#[case::users("users")]
#[case::version("version")]
#[case::whoami("whoami")]
fn every_family_command_is_mounted(#[case] command: &str) {
    let root = root_command(&FakeEnvironment::default());
    let mounted = root
        .get_subcommands()
        .map(Command::get_name)
        .any(|name| name == command);
    assert!(mounted, "{command} is not mounted");
}

#[test]
fn every_known_agent_is_mounted() {
    let root = root_command(&FakeEnvironment::default());
    let names: Vec<&str> = root.get_subcommands().map(Command::get_name).collect();
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
    let mounted = root
        .get_subcommands()
        .map(Command::get_name)
        .any(|name| name == "statusline");
    assert!(mounted);
}
