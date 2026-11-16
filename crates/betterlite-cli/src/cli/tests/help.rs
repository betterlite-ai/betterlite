use crate::cli::tests::support::{FakeEnvironment, Harness};

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

pub(super) fn listed_commands(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut inside = false;
    for line in help.lines() {
        let trimmed = line.trim();
        if trimmed == "Commands:" {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        if trimmed.is_empty() || trimmed == "Options:" {
            break;
        }
        if let Some(name) = trimmed.split_whitespace().next() {
            names.push(name.to_owned());
        }
    }
    names
}

fn help_output() -> String {
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&["--help"]);
    assert_eq!(code, 0);
    harness.stdout()
}

#[test]
fn help_lists_every_family_command_alphabetically() {
    let names = listed_commands(&help_output());
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "the listing must be alphabetical");
    for command in FAMILY_COMMANDS {
        assert!(
            names.iter().any(|name| name == command),
            "{command} is missing"
        );
    }
}

#[test]
fn help_omits_the_statusline_command() {
    let names = listed_commands(&help_output());
    assert!(!names.iter().any(|name| name == "statusline"));
}

#[test]
fn help_mentions_the_global_options() {
    let help = help_output();
    assert!(help.contains("--base-url"));
    assert!(help.contains("--api-key"));
    assert!(help.contains("--version"));
}

#[test]
fn help_does_not_show_an_uncallable_help_command() {
    let names = listed_commands(&help_output());
    assert!(!names.iter().any(|name| name == "help"));
}
