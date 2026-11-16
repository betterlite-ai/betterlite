use crate::cli::root::root_command;
use crate::cli::tests::support::FakeEnvironment;

const MINIMAL_ARGV: [&[&str]; 26] = [
    &["login"],
    &["logout"],
    &["whoami"],
    &["auth", "print-token"],
    &["up"],
    &["down"],
    &["models", "list"],
    &["credentials", "list"],
    &["users", "list"],
    &["teams", "list"],
    &["model-groups", "list"],
    &["keys", "list"],
    &["encryption", "migrate"],
    &["config", "get"],
    &["configure"],
    &["reconfigure"],
    &["unconfigure"],
    &["autoroute", "configure"],
    &["chat"],
    &["http", "request", "GET", "/v1/models"],
    &["debug", "claude"],
    &["debug", "install-claude-command"],
    &["statusline"],
    &["version"],
    &["claude"],
    &["codex"],
];

#[test]
fn every_family_command_parses_a_minimal_argv() {
    let environment = FakeEnvironment::default();
    for argv in MINIMAL_ARGV {
        let mut full: Vec<&str> = vec!["lite"];
        full.extend_from_slice(argv);
        let matches = root_command(&environment)
            .try_get_matches_from(full)
            .unwrap_or_else(|error| panic!("{argv:?} must parse: {error}"));
        let (name, _sub) = matches
            .subcommand()
            .unwrap_or_else(|| panic!("{argv:?} must reach a subcommand"));
        assert_eq!(
            name, argv[0],
            "the parsed subcommand must be the one requested"
        );
    }
}

#[test]
fn every_remaining_agent_parses_a_minimal_argv() {
    let environment = FakeEnvironment::default();
    for argv in [&["opencode"][..], &["pi"][..]] {
        let mut full: Vec<&str> = vec!["lite"];
        full.extend_from_slice(argv);
        let matches = root_command(&environment)
            .try_get_matches_from(full)
            .unwrap_or_else(|error| panic!("{argv:?} must parse: {error}"));
        assert!(matches.subcommand().is_some());
    }
}

#[test]
fn agent_forwarded_arguments_parse() {
    let environment = FakeEnvironment::default();
    let matches = root_command(&environment)
        .try_get_matches_from(["lite", "codex", "--skip-verify", "--", "exec", "hello"])
        .unwrap();
    let (name, sub) = matches.subcommand().unwrap();
    assert_eq!(name, "codex");
    let forwarded: Vec<String> = sub
        .get_many::<String>("args")
        .map(|values| values.cloned().collect())
        .unwrap_or_default();
    assert_eq!(forwarded, [String::from("exec"), String::from("hello")]);
}
