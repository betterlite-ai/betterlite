use rstest::rstest;

use super::support::json;
use crate::datadog_llm_obs::constants::{
    MAX_PARSED_TOOL_ARGUMENT_CHARS, SAFE_REDACTED_MESSAGE_ROLES, safe_message_role,
};

#[rstest]
#[case::agent("agent", "agent")]
#[case::assistant("assistant", "assistant")]
#[case::developer("developer", "developer")]
#[case::function("function", "function")]
#[case::model("model", "model")]
#[case::system("system", "system")]
#[case::tool("tool", "tool")]
#[case::user("user", "user")]
#[case::unknown("bot", "")]
#[case::different_case("Agent", "")]
fn message_roles_are_kept_only_when_recognized(#[case] role: &str, #[case] expected: &str) {
    let value = json(&format!("\"{role}\""));
    assert_eq!(safe_message_role(Some(&value)), expected);
}

#[test]
fn message_roles_drop_values_that_are_not_strings() {
    assert_eq!(safe_message_role(Some(&json("7"))), "");
    assert_eq!(safe_message_role(Some(&json("null"))), "");
    assert_eq!(safe_message_role(None), "");
}

#[test]
fn the_parsed_argument_bound_is_256_kibibytes() {
    assert_eq!(MAX_PARSED_TOOL_ARGUMENT_CHARS, 256 * 1024);
}

#[test]
fn the_safe_role_set_holds_the_eight_roles_litellm_declares() {
    assert_eq!(
        SAFE_REDACTED_MESSAGE_ROLES,
        [
            "agent",
            "assistant",
            "developer",
            "function",
            "model",
            "system",
            "tool",
            "user"
        ]
    );
}
