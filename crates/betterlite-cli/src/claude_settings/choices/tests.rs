#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;

use super::{ModelChoice, STARTING_MODEL_ROLE, StaticToken};

#[test]
fn starting_model_role_names_the_row_claude_code_starts_on() {
    assert_eq!(
        STARTING_MODEL_ROLE,
        "the /model picker's default row, the model Claude Code starts and resumes on"
    );
}

#[rstest]
#[case::keep_model(ModelChoice::KeepModel, None)]
#[case::unpin_model(ModelChoice::UnpinModel, None)]
#[case::start_on(
    ModelChoice::StartOn("claude-sonnet-4".to_owned()),
    Some("claude-sonnet-4")
)]
fn starting_model_only_reports_a_start_pin(
    #[case] choice: ModelChoice,
    #[case] expected: Option<&str>,
) {
    assert_eq!(choice.starting_model(), expected);
}

#[test]
fn static_token_keeps_the_token_out_of_debug_output() {
    let credential = StaticToken {
        token: SecretString::from("sk-claude-secret"),
    };
    let printed = format!("{credential:?}");
    assert!(!printed.contains("sk-claude-secret"));
    assert!(printed.contains("REDACTED"));
}
