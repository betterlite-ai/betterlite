use rstest::rstest;

use super::super::{SystemMessage, system_message};
use super::support::json;

#[rstest]
#[case::text(r#""guide""#)]
#[case::blocks(r#"[{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]"#)]
fn a_nonempty_system_becomes_a_system_message(#[case] system: &str) {
    assert_eq!(
        system_message(&json(system)),
        Some(SystemMessage {
            role: "system",
            content: json(system)
        })
    );
}

#[rstest]
#[case::empty_text(r#""""#)]
#[case::empty_list("[]")]
#[case::null("null")]
#[case::object(r#"{"type": "text"}"#)]
#[case::number("7")]
#[case::boolean("true")]
fn anything_else_has_no_system_message(#[case] system: &str) {
    assert_eq!(system_message(&json(system)), None);
}
