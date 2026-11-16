use rstest::rstest;

use crate::cli::dispatch::forwarded_args;
use crate::cli::root::root_command;
use crate::cli::tests::support::FakeEnvironment;

#[rstest]
#[case::login(&["login"])]
#[case::logout(&["logout"])]
#[case::whoami(&["whoami"])]
#[case::auth_print_token(&["auth", "print-token"])]
#[case::up(&["up"])]
#[case::down(&["down"])]
#[case::models_list(&["models", "list"])]
#[case::credentials_list(&["credentials", "list"])]
#[case::users_list(&["users", "list"])]
#[case::teams_list(&["teams", "list"])]
#[case::model_groups_list(&["model-groups", "list"])]
#[case::keys_list(&["keys", "list"])]
#[case::encryption_migrate(&["encryption", "migrate"])]
#[case::config_get(&["config", "get"])]
#[case::configure(&["configure"])]
#[case::reconfigure(&["reconfigure"])]
#[case::unconfigure(&["unconfigure"])]
#[case::autoroute_configure(&["autoroute", "configure"])]
#[case::chat(&["chat"])]
#[case::http_request(&["http", "request", "GET", "/v1/models"])]
#[case::debug_claude(&["debug", "claude"])]
#[case::debug_install_claude_command(&["debug", "install-claude-command"])]
#[case::statusline(&["statusline"])]
#[case::version(&["version"])]
#[case::claude(&["claude"])]
#[case::codex(&["codex"])]
#[case::opencode(&["opencode"])]
#[case::pi(&["pi"])]
fn every_mounted_command_parses_a_minimal_argv(#[case] argv: &[&str]) {
    let environment = FakeEnvironment::default();
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

#[test]
fn agent_forwarded_arguments_parse() {
    let environment = FakeEnvironment::default();
    let matches = root_command(&environment)
        .try_get_matches_from(["lite", "codex", "--skip-verify", "--", "exec", "hello"])
        .unwrap();
    let (name, sub) = matches.subcommand().unwrap();
    assert_eq!(name, "codex");
    assert_eq!(
        forwarded_args(sub),
        [String::from("exec"), String::from("hello")]
    );
}
