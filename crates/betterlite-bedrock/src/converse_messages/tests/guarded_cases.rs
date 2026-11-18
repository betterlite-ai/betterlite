use crate::converse_messages::error::ConverseMessageError;
use crate::converse_messages::guarded::guard_trailing_user_turns;

use super::support::{expected, messages, params, render};

const GUARDRAIL: &str = r#"{"guardrailConfig": null}"#;
const GUARDRAIL_FALSE: &str = r#"{"guardrailConfig": false}"#;

pub(super) fn consecutive_guarded_user_text() {
    key_presence_enables_conversion();
    only_the_trailing_run_converts();
    existing_guarded_text_skips_the_message();
    string_and_missing_content();
    a_text_part_without_text_is_rejected();
}

fn key_presence_enables_conversion() {
    let history = r#"[
        {"role": "system", "content": "rules"},
        {"role": "user", "content": "first"},
        {"role": "assistant", "content": "reply"},
        {"role": "user", "content": "last"}
    ]"#;
    let unchanged =
        guard_trailing_user_turns(&messages(history), &params("{}")).expect("the messages convert");
    assert_eq!(render(&unchanged), expected(history));
    let guarded = r#"[
        {"role": "system", "content": "rules"},
        {"role": "user", "content": "first"},
        {"role": "assistant", "content": "reply"},
        {"role": "user", "content": [{"type": "guarded_text", "text": "last"}]}
    ]"#;
    for optional in [GUARDRAIL, GUARDRAIL_FALSE] {
        let converted = guard_trailing_user_turns(&messages(history), &params(optional))
            .expect("the messages convert");
        assert_eq!(render(&converted), expected(guarded), "{optional}");
    }
}

fn only_the_trailing_run_converts() {
    let converted = guard_trailing_user_turns(
        &messages(
            r#"[
                {"role": "user", "content": "earlier"},
                {"role": "assistant", "content": "reply"},
                {"role": "user", "content": "one"},
                {"role": "user", "content": [{"type": "text", "text": "two", "cache_control": {"type": "ephemeral"}, "extra": 1}, {"type": "image", "source": {"bytes": "AA=="}}]}
            ]"#,
        ),
        &params(GUARDRAIL),
    )
    .expect("the messages convert");
    assert_eq!(
        render(&converted),
        expected(
            r#"[
                {"role": "user", "content": "earlier"},
                {"role": "assistant", "content": "reply"},
                {"role": "user", "content": [{"type": "guarded_text", "text": "one"}]},
                {"role": "user", "content": [{"type": "guarded_text", "text": "two"}, {"type": "image", "source": {"bytes": "AA=="}}]}
            ]"#
        )
    );
    let assistant_last = r#"[
        {"role": "user", "content": "question"},
        {"role": "assistant", "content": "answer"}
    ]"#;
    let kept = guard_trailing_user_turns(&messages(assistant_last), &params(GUARDRAIL))
        .expect("the messages convert");
    assert_eq!(render(&kept), expected(assistant_last));
}

fn existing_guarded_text_skips_the_message() {
    let history = r#"[
        {"role": "user", "content": [{"type": "text", "text": "keep", "cache_control": {"type": "ephemeral"}}, {"type": "guarded_text", "text": "already"}, {"type": "text", "text": "also"}]}
    ]"#;
    let kept = guard_trailing_user_turns(&messages(history), &params(GUARDRAIL))
        .expect("the messages convert");
    assert_eq!(render(&kept), expected(history));
}

fn string_and_missing_content() {
    let converted = guard_trailing_user_turns(
        &messages(
            r#"[
                {"role": "user", "content": ""},
                {"role": "user", "content": []},
                {"role": "user", "content": null},
                {"role": "user", "name": "alice", "extra": true}
            ]"#,
        ),
        &params(GUARDRAIL),
    )
    .expect("the messages convert");
    assert_eq!(
        render(&converted),
        expected(
            r#"[
                {"role": "user", "content": [{"type": "guarded_text", "text": ""}]},
                {"role": "user", "content": []},
                {"role": "user", "content": []},
                {"role": "user", "content": [], "name": "alice", "extra": true}
            ]"#
        )
    );
}

fn a_text_part_without_text_is_rejected() {
    let error = guard_trailing_user_turns(
        &messages(
            r#"[
                {"role": "user", "content": "fine"},
                {"role": "user", "content": [{"type": "text", "text": "x"}, {"type": "text"}]}
            ]"#,
        ),
        &params(GUARDRAIL),
    )
    .expect_err("a text part without a text value is rejected");
    assert!(
        matches!(
            error,
            ConverseMessageError::TextPartWithoutText { message: 1 }
        ),
        "{error}"
    );
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        error.to_string(),
        "messages[1].content has a text part without a text value"
    );
}
