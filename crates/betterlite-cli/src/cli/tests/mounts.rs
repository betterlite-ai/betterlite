use crate::agent_launch::KNOWN_AGENTS;
use crate::cli::root::root_command;
use crate::cli::tests::support::FakeEnvironment;

const FAMILY_COMMANDS: [&str; 22] = [
    "auth",
    "autoroute",
    "chat",
    "config",
    "configure",
    "credentials",
    "debug",
    "down",
    "encryption",
    "http",
    "keys",
    "login",
    "logout",
    "model-groups",
    "models",
    "reconfigure",
    "teams",
    "unconfigure",
    "up",
    "users",
    "version",
    "whoami",
];

#[test]
fn every_family_command_is_mounted() {
    let root = root_command(&FakeEnvironment::default());
    let names: Vec<&str> = root
        .get_subcommands()
        .map(|subcommand| subcommand.get_name())
        .collect();
    for command in FAMILY_COMMANDS {
        assert!(names.contains(&command), "{command} is not mounted");
    }
}

#[test]
fn every_known_agent_is_mounted() {
    let root = root_command(&FakeEnvironment::default());
    let names: Vec<&str> = root
        .get_subcommands()
        .map(|subcommand| subcommand.get_name())
        .collect();
    for (binary, _display, _profiles, _hidden) in KNOWN_AGENTS {
        assert!(names.contains(&binary), "{binary} is not mounted");
    }
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
