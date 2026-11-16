use rstest::rstest;

use crate::cli::tests::support::{FAMILY_COMMANDS, FakeEnvironment, Harness};

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

#[rstest]
fn help_lists_every_family_command_alphabetically(#[values(FAMILY_COMMANDS)] command: &str) {
    let names = listed_commands(&help_output());
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "the listing must be alphabetical");
    assert!(
        names.iter().any(|name| name == command),
        "{command} is missing"
    );
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
