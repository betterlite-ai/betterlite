use crate::arguments::json::{Json, Object};
use crate::cache_placement::messages_route::apply_messages_route;
use crate::cache_placement::system::SystemPrompt;
use crate::cache_placement::tests::support::{json, messages, points, wire};

fn run(
    messages_wire: &str,
    system_wire: Option<&str>,
    points_wire: &str,
    openai: bool,
    external: usize,
) -> Json {
    let conversation = messages(messages_wire);
    let system: Option<SystemPrompt> = system_wire.map(|text| serde_json::from_str(text).unwrap());
    let injection = apply_messages_route(
        &conversation,
        system.as_ref(),
        &points(points_wire),
        openai,
        external,
    );
    let mut result = Object::new();
    result.insert("messages".to_owned(), wire(&injection.messages));
    result.insert("system".to_owned(), wire(&injection.system));
    result.insert("remaining".to_owned(), wire(&injection.remaining));
    Json::Object(result)
}

const USER_POINT: &str =
    r#"[{"location": "message", "role": "user", "index": null, "control": null}]"#;
const SYSTEM_POINT: &str =
    r#"[{"location": "message", "role": "system", "index": null, "control": null}]"#;
const TOOL_POINT: &str = r#"[{"location": "tool_config", "control": null}]"#;

fn marked(role: &str, label: &str) -> String {
    format!(
        r#"{{"role": "{role}", "content": [{{"type": "text", "text": "{label}"}}], "cache_control": {{}}}}"#
    )
}

fn converted(role: &str, label: &str) -> String {
    format!(r#"{{"role": "{role}", "content": [{{"type": "text", "text": "{label}"}}]}}"#)
}

fn block_marked(role: &str, label: &str) -> String {
    format!(
        r#"{{"role": "{role}", "content": [{{"type": "text", "text": "{label}", "cache_control": {{"type": "ephemeral"}}}}]}}"#
    )
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

fn marked_users(labels: &[&str]) -> String {
    let items: Vec<String> = labels.iter().map(|label| marked("user", label)).collect();
    list(&items)
}

#[test]
fn no_points_change_nothing_and_convert_no_content() {
    let input = r#"[{"role": "user", "content": "hi"}, {"role": "assistant", "content": "yo"}]"#;
    let expected = json(
        r#"{"messages": [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "yo"}], "system": "sys", "remaining": []}"#,
    );
    assert_eq!(run(input, Some(r#""sys""#), "[]", false, 0), expected);
}

#[test]
fn an_injection_converts_every_string_content() {
    let input = r#"[{"role": "user", "content": "hi"}, {"role": "assistant", "content": "yo"}]"#;
    let expected_messages = list(&[block_marked("user", "hi"), converted("assistant", "yo")]);
    let expected = json(&format!(
        r#"{{"messages": {expected_messages}, "system": null, "remaining": []}}"#
    ));
    assert_eq!(run(input, None, USER_POINT, false, 0), expected);
}

#[test]
fn openai_converts_and_marks_with_the_breakpoint() {
    let input = r#"[{"role": "user", "content": "hi"}]"#;
    let expected = json(
        r#"{"messages": [{"role": "user", "content": [{"type": "text", "text": "hi", "prompt_cache_breakpoint": {"mode": "explicit"}}]}], "system": null, "remaining": []}"#,
    );
    assert_eq!(run(input, None, USER_POINT, true, 0), expected);
}

#[test]
fn a_system_point_converts_a_text_system_first() {
    let input = r#"[{"role": "user", "content": "hi"}]"#;
    let expected = json(
        r#"{"messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}],
            "system": [{"type": "text", "text": "sys", "cache_control": {"type": "ephemeral"}}],
            "remaining": []}"#,
    );
    assert_eq!(
        run(input, Some(r#""sys""#), SYSTEM_POINT, false, 0),
        expected
    );
}

#[test]
fn an_openai_system_point_uses_the_breakpoint_marker() {
    let expected = json(
        r#"{"messages": [], "system": [{"type": "text", "text": "sys", "prompt_cache_breakpoint": {"mode": "explicit"}}], "remaining": []}"#,
    );
    assert_eq!(run("[]", Some(r#""sys""#), SYSTEM_POINT, true, 0), expected);
}

#[test]
fn a_system_list_marks_only_its_last_block() {
    let system = r#"[{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]"#;
    let expected = json(
        r#"{"messages": [], "system": [{"type": "text", "text": "a"}, {"type": "text", "text": "b", "cache_control": {"type": "ephemeral"}}], "remaining": []}"#,
    );
    assert_eq!(run("[]", Some(system), SYSTEM_POINT, false, 0), expected);
}

#[test]
fn a_marked_system_is_left_alone() {
    let system =
        r#"[{"type": "text", "text": "a", "cache_control": {"type": "ephemeral", "ttl": "1h"}}]"#;
    let expected = json(
        r#"{"messages": [], "system": [{"type": "text", "text": "a", "cache_control": {"type": "ephemeral", "ttl": "1h"}}], "remaining": []}"#,
    );
    assert_eq!(run("[]", Some(system), SYSTEM_POINT, false, 0), expected);
}

#[test]
fn a_system_point_without_a_system_is_dropped() {
    let expected = json(
        r#"{"messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}], "system": null, "remaining": []}"#,
    );
    assert_eq!(
        run(
            r#"[{"role": "user", "content": "hi"}]"#,
            None,
            SYSTEM_POINT,
            false,
            0
        ),
        expected
    );
}

#[test]
fn a_system_point_reserves_its_slot_before_messages() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "user", "content": "b", "cache_control": {}},
        {"role": "user", "content": "c", "cache_control": {}}
    ]"#;
    let expected = json(&format!(
        r#"{{"messages": {}, "system": [{{"type": "text", "text": "sys", "cache_control": {{"type": "ephemeral"}}}}], "remaining": []}}"#,
        marked_users(&["a", "b", "c"])
    ));
    assert_eq!(
        run(input, Some(r#""sys""#), SYSTEM_POINT, false, 0),
        expected
    );
}

#[test]
fn a_full_budget_leaves_the_system_alone() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "user", "content": "b", "cache_control": {}},
        {"role": "user", "content": "c", "cache_control": {}},
        {"role": "user", "content": "d", "cache_control": {}}
    ]"#;
    let expected = json(&format!(
        r#"{{"messages": {}, "system": "sys", "remaining": []}}"#,
        marked_users(&["a", "b", "c", "d"])
    ));
    assert_eq!(
        run(input, Some(r#""sys""#), SYSTEM_POINT, false, 0),
        expected
    );
}

#[test]
fn a_tool_config_point_reserves_a_slot() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "user", "content": "b", "cache_control": {}},
        {"role": "user", "content": "c", "cache_control": {}}
    ]"#;
    let points = r#"[{"location": "message", "role": "system", "index": null, "control": null}, {"location": "tool_config", "control": null}]"#;
    let expected = json(&format!(
        r#"{{"messages": {}, "system": "sys", "remaining": [{{"location": "tool_config", "control": null}}]}}"#,
        marked_users(&["a", "b", "c"])
    ));
    assert_eq!(run(input, Some(r#""sys""#), points, false, 0), expected);
}

#[test]
fn a_tool_config_point_is_dropped_when_the_wire_is_full() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "user", "content": "b", "cache_control": {}},
        {"role": "user", "content": "c", "cache_control": {}},
        {"role": "user", "content": "d", "cache_control": {}}
    ]"#;
    let expected = json(&format!(
        r#"{{"messages": {}, "system": null, "remaining": []}}"#,
        marked_users(&["a", "b", "c", "d"])
    ));
    assert_eq!(run(input, None, TOOL_POINT, false, 0), expected);
}

#[test]
fn openai_keeps_a_tool_config_point_and_reserves_nothing() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "user", "content": "b", "cache_control": {}},
        {"role": "user", "content": "c", "cache_control": {}},
        {"role": "user", "content": "d", "cache_control": {}}
    ]"#;
    let expected = json(&format!(
        r#"{{"messages": {}, "system": null, "remaining": [{{"location": "tool_config", "control": null}}]}}"#,
        marked_users(&["a", "b", "c", "d"])
    ));
    assert_eq!(run(input, None, TOOL_POINT, true, 0), expected);
}

#[test]
fn external_breakpoints_shrink_the_budget() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "user", "content": "b", "cache_control": {}}
    ]"#;
    let expected = json(&format!(
        r#"{{"messages": {}, "system": "sys", "remaining": []}}"#,
        marked_users(&["a", "b"])
    ));
    assert_eq!(
        run(input, Some(r#""sys""#), SYSTEM_POINT, false, 2),
        expected
    );
}

#[test]
fn one_external_breakpoint_still_leaves_room_for_the_system() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "user", "content": "b", "cache_control": {}}
    ]"#;
    let expected = json(&format!(
        r#"{{"messages": {}, "system": [{{"type": "text", "text": "sys", "cache_control": {{"type": "ephemeral"}}}}], "remaining": []}}"#,
        marked_users(&["a", "b"])
    ));
    assert_eq!(
        run(input, Some(r#""sys""#), SYSTEM_POINT, false, 1),
        expected
    );
}

#[test]
fn an_index_point_marks_the_named_message() {
    let input = r#"[{"role": "user", "content": "a"}, {"role": "assistant", "content": "b"}]"#;
    let expected_messages = list(&[converted("user", "a"), block_marked("assistant", "b")]);
    let expected = json(&format!(
        r#"{{"messages": {expected_messages}, "system": null, "remaining": []}}"#
    ));
    let points = r#"[{"location": "message", "role": null, "index": 1, "control": null}]"#;
    assert_eq!(run(input, None, points, false, 0), expected);
}

#[test]
fn an_assistant_point_is_skipped_in_openai_mode() {
    let input = r#"[{"role": "assistant", "content": "b"}]"#;
    let expected = json(
        r#"{"messages": [{"role": "assistant", "content": [{"type": "text", "text": "b"}]}], "system": null, "remaining": []}"#,
    );
    let points = r#"[{"location": "message", "role": null, "index": 0, "control": null}]"#;
    assert_eq!(run(input, None, points, true, 0), expected);
}

#[test]
fn a_system_mark_takes_a_slot_before_a_message_point() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "assistant", "content": "b", "cache_control": {}},
        {"role": "user", "content": "c", "cache_control": {}},
        {"role": "user", "content": "d"}
    ]"#;
    let points = r#"[{"location": "message", "role": "user", "index": null, "control": null}, {"location": "message", "role": "system", "index": null, "control": null}]"#;
    let expected_messages = list(&[
        marked("user", "a"),
        marked("assistant", "b"),
        marked("user", "c"),
        converted("user", "d"),
    ]);
    let expected = json(&format!(
        r#"{{"messages": {expected_messages}, "system": [{{"type": "text", "text": "sys", "cache_control": {{"type": "ephemeral"}}}}], "remaining": []}}"#
    ));
    assert_eq!(run(input, Some(r#""sys""#), points, false, 0), expected);
}
