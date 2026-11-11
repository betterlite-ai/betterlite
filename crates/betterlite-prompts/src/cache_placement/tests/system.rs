use crate::arguments::json::Json;
use crate::cache_placement::system::{SystemPrompt, marked_system};
use crate::cache_placement::tests::support::{json, wire};

const CONTROL: &str = r#"{"type": "ephemeral"}"#;

fn marked(wire_text: &str, control_text: &str, openai_dialect: bool) -> (Json, bool) {
    let system: SystemPrompt = serde_json::from_str(wire_text).unwrap();
    let (marked, added) = marked_system(&system, &json(control_text), openai_dialect);
    (wire(&marked), added)
}

#[test]
fn a_text_system_becomes_one_marked_text_block() {
    let (result, added) = marked(r#""be brief""#, CONTROL, false);
    assert_eq!(
        result,
        json(r#"[{"type": "text", "text": "be brief", "cache_control": {"type": "ephemeral"}}]"#)
    );
    assert!(added);
}

#[test]
fn an_openai_text_system_gets_the_breakpoint_marker() {
    let (result, added) = marked(r#""be brief""#, CONTROL, true);
    assert_eq!(
        result,
        json(
            r#"[{"type": "text", "text": "be brief", "prompt_cache_breakpoint": {"mode": "explicit"}}]"#
        )
    );
    assert!(added);
}

#[test]
fn a_block_system_marks_only_its_last_block() {
    let (result, added) = marked(
        r#"[{"type": "text", "text": "a"}, {"type": "text", "text": "b"}, {"type": "text", "text": "c"}]"#,
        CONTROL,
        false,
    );
    assert_eq!(
        result,
        json(
            r#"[{"type": "text", "text": "a"}, {"type": "text", "text": "b"}, {"type": "text", "text": "c", "cache_control": {"type": "ephemeral"}}]"#
        )
    );
    assert!(added);
}

#[test]
fn a_block_system_keeps_its_other_fields() {
    let (result, added) = marked(
        r#"[{"type": "text", "text": "a", "extra": "x"}]"#,
        r#"{"type": "ephemeral"}"#,
        false,
    );
    assert_eq!(
        result,
        json(
            r#"[{"type": "text", "text": "a", "extra": "x", "cache_control": {"type": "ephemeral"}}]"#
        )
    );
    assert!(added);
}

#[test]
fn an_empty_block_system_gets_no_mark() {
    let (result, added) = marked("[]", CONTROL, false);
    assert_eq!(result, json("[]"));
    assert!(!added);
}
