use rstest::rstest;

use crate::cli::tests::support::{FakeEnvironment, Harness, unused_base_url};

#[rstest]
#[case::models(&["models", "list"])]
#[case::credentials(&["credentials", "list"])]
#[case::users(&["users", "list"])]
#[case::teams(&["teams", "list"])]
#[case::model_groups(&["model-groups", "list"])]
#[case::keys(&["keys", "list"])]
#[case::encryption(&["encryption", "migrate"])]
#[case::http(&["http", "request", "GET", "/v1/models"])]
#[case::debug(&["debug", "claude"])]
#[case::config(&["config", "set", "nope", "nope"])]
#[case::down(&["down"])]
#[case::statusline(&["statusline"])]
#[case::logout(&["logout"])]
#[case::login(&["login"])]
#[case::up(&["up"])]
#[case::auth(&["auth", "print-token"])]
#[case::agent(&["claude"])]
#[case::configure(&["configure"])]
#[case::reconfigure(&["reconfigure"])]
#[case::unconfigure(&["unconfigure"])]
#[case::autoroute(&["autoroute", "configure"])]
fn a_command_that_cannot_run_reports_one_error(#[case] command: &[&str]) {
    let base_url = unused_base_url();
    let mut argv: Vec<&str> = vec!["--base-url", &base_url];
    argv.extend_from_slice(command);
    let mut harness = Harness::new(FakeEnvironment::default());
    let code = harness.run(&argv);
    assert_ne!(
        code,
        0,
        "{command:?} must fail without a proxy or a home: {}",
        harness.stdout()
    );
    assert!(
        !harness.stderr().is_empty(),
        "{command:?} must report why it failed"
    );
}
