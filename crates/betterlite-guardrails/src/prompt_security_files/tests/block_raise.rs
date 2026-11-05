#![cfg(test)]

use reqwest::StatusCode;
use rstest::rstest;

use super::shared::sanitizer;
use crate::prompt_security_files::error::FileSanitizationError;
use crate::prompt_security_files::wire::{Action, SanitizeResult};

fn result(action: Action) -> SanitizeResult {
    SanitizeResult {
        action,
        content: None,
        violations: vec!["secret".to_owned(), "pii".to_owned()],
    }
}

#[rstest]
#[case::block(Action::Block, true, true)]
#[case::block_with_modify_allowed(Action::Block, false, true)]
#[case::modify_blocks_by_default(Action::Modify, true, true)]
#[case::modify_allowed(Action::Modify, false, false)]
#[case::allow(Action::Allow, true, false)]
#[case::unrecognized(Action::Unrecognized, true, false)]
fn block_raise_follows_action_and_block_on_file_modify(
    #[case] action: Action,
    #[case] block_on_file_modify: bool,
    #[case] raises: bool,
) {
    let mut guardrail = sanitizer("http://unused.test");
    guardrail.settings.block_on_file_modify = block_on_file_modify;
    let outcome = guardrail.raise_if_file_blocked(&result(action), "File");
    assert_eq!(outcome.is_err(), raises);
}

#[test]
fn block_raise_is_a_400_naming_the_resource_and_violations() {
    let error = sanitizer("http://unused.test")
        .raise_if_file_blocked(&result(Action::Block), "Document")
        .expect_err("blocked");
    assert!(matches!(error, FileSanitizationError::Blocked { .. }));
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.to_string(),
        "Document blocked by Prompt Security. Violations: secret, pii"
    );
}
