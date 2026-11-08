use crate::anthropic_invokes::cache_control::add_cache_control_to_content;

use super::object;

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_cache_control_object_is_added_to_the_element() {
    assert_eq!(
        add_cache_control_to_content(
            object(r#"{"type":"text","text":"a"}"#),
            &object(r#"{"cache_control":{"type":"ephemeral"}}"#),
        ),
        object(r#"{"type":"text","text":"a","cache_control":{"type":"ephemeral"}}"#)
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_an_empty_cache_control_object_is_added() {
    assert_eq!(
        add_cache_control_to_content(
            object(r#"{"type":"text"}"#),
            &object(r#"{"cache_control":{}}"#),
        ),
        object(r#"{"type":"text","cache_control":{}}"#)
    );
}

#[rstest::rstest]
#[case::text(r#""ephemeral""#)]
#[case::list(r#"[{"type":"ephemeral"}]"#)]
#[case::number("5")]
#[case::boolean("true")]
#[case::null("null")]
#[case::missing("")]
fn anthropic_invokes_behavior_tests_behavior_a_non_object_cache_control_is_ignored(
    #[case] cache_control: &str,
) {
    let original = if cache_control.is_empty() {
        object(r#"{"type":"text"}"#)
    } else {
        object(&format!(r#"{{"cache_control":{cache_control}}}"#))
    };
    assert_eq!(
        add_cache_control_to_content(object(r#"{"type":"text"}"#), &original),
        object(r#"{"type":"text"}"#)
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_an_existing_cache_control_is_overwritten() {
    assert_eq!(
        add_cache_control_to_content(
            object(r#"{"type":"text","cache_control":{"type":"ephemeral"}}"#),
            &object(r#"{"cache_control":{"type":"ephemeral","ttl":"1h"}}"#),
        ),
        object(r#"{"type":"text","cache_control":{"type":"ephemeral","ttl":"1h"}}"#)
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_cache_control_keys_are_copied_verbatim() {
    assert_eq!(
        add_cache_control_to_content(
            object(r#"{"type":"text"}"#),
            &object(r#"{"cache_control":{"type":"ephemeral","zzz":1}}"#),
        ),
        object(r#"{"type":"text","cache_control":{"type":"ephemeral","zzz":1}}"#)
    );
}

#[test]
fn anthropic_invokes_behavior_tests_behavior_the_element_keeps_every_source_field() {
    let element = object(r#"{"type":"tool_use","id":"c1","name":"x","input":{"a":1}}"#);
    let original = object(r#"{"id":"other","cache_control":{"type":"ephemeral"},"extra":"kept"}"#);
    assert_eq!(
        add_cache_control_to_content(element, &original),
        object(
            r#"{"type":"tool_use","id":"c1","name":"x","input":{"a":1},"cache_control":{"type":"ephemeral"}}"#
        )
    );
}
