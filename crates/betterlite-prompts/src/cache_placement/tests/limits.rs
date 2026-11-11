use crate::arguments::json::Json;
use crate::cache_accounting::points::MessageInjectionPoint;
use crate::cache_placement::message_injections::apply_message_injections;
use crate::cache_placement::tests::support::{
    capture_warnings, json, message_points, messages, wire,
};

fn apply(messages_wire: &str, points_wire: &str, max_blocks: usize, openai: bool) -> Json {
    let conversation = messages(messages_wire);
    let points = message_points(points_wire);
    let borrowed: Vec<&MessageInjectionPoint> = points.iter().collect();
    wire(&apply_message_injections(
        &borrowed,
        &conversation,
        max_blocks,
        openai,
    ))
}

const FOUR_MARKS: &str = r#"[
    {"role": "user", "content": "a", "cache_control": {}},
    {"role": "assistant", "content": "b", "cache_control": {}},
    {"role": "user", "content": "c", "cache_control": {}},
    {"role": "assistant", "content": "d", "cache_control": {}}
]"#;

#[test]
fn a_full_budget_skips_every_point() {
    let expected = json(FOUR_MARKS);
    let result = apply(FOUR_MARKS, r#"[{"role": "user"}]"#, 4, false);
    assert_eq!(result, expected);
}

#[test]
fn a_client_mark_is_kept_and_uses_a_slot() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {"type": "ephemeral", "ttl": "1h"}},
        {"role": "user", "content": "b"}
    ]"#;
    let expected = json(
        r#"[
        {"role": "user", "content": "a", "cache_control": {"type": "ephemeral", "ttl": "1h"}},
        {"role": "user", "content": "b", "cache_control": {"type": "ephemeral"}}
    ]"#,
    );
    assert_eq!(apply(input, r#"[{"role": "user"}]"#, 2, false), expected);
}

#[test]
fn a_client_mark_is_never_overwritten_even_with_a_slot_left() {
    let input = r#"[{"role": "user", "content": "a", "cache_control": {"type": "ephemeral", "ttl": "1h"}}]"#;
    let expected = json(input);
    assert_eq!(apply(input, r#"[{"role": "user"}]"#, 4, false), expected);
}

#[test]
fn earlier_points_win_when_the_budget_fills() {
    let input = r#"[
        {"role": "user", "content": "a"},
        {"role": "assistant", "content": "b"}
    ]"#;
    let expected = json(
        r#"[
        {"role": "user", "content": "a", "cache_control": {"type": "ephemeral"}},
        {"role": "assistant", "content": "b"}
    ]"#,
    );
    let points = r#"[{"role": "user"}, {"role": "assistant"}]"#;
    assert_eq!(apply(input, points, 1, false), expected);
}

#[test]
fn a_repeated_target_spends_one_slot() {
    let input = r#"[
        {"role": "user", "content": "a"},
        {"role": "user", "content": "b"}
    ]"#;
    let expected = json(
        r#"[
        {"role": "user", "content": "a", "cache_control": {"type": "ephemeral"}},
        {"role": "user", "content": "b", "cache_control": {"type": "ephemeral"}}
    ]"#,
    );
    let points = r#"[{"index": 0}, {"index": 0}, {"index": 1}]"#;
    assert_eq!(apply(input, points, 2, false), expected);
}

#[test]
fn a_skipped_assistant_spends_no_slot_in_openai_mode() {
    let input = r#"[
        {"role": "assistant", "content": "a"},
        {"role": "user", "content": "b"}
    ]"#;
    let expected = json(
        r#"[
        {"role": "assistant", "content": "a"},
        {"role": "user", "content": [{"type": "text", "text": "b", "prompt_cache_breakpoint": {"mode": "explicit"}}]}
    ]"#,
    );
    let points = r#"[{"role": "assistant"}, {"role": "user"}]"#;
    assert_eq!(apply(input, points, 1, true), expected);
}

#[test]
fn an_ineligible_block_spends_no_slot_in_openai_mode() {
    let input = r#"[
        {"role": "user", "content": [{"type": "video", "source": "x"}]},
        {"role": "user", "content": [{"type": "text", "text": "b"}]}
    ]"#;
    let expected = json(
        r#"[
        {"role": "user", "content": [{"type": "video", "source": "x"}]},
        {"role": "user", "content": [{"type": "text", "text": "b", "prompt_cache_breakpoint": {"mode": "explicit"}}]}
    ]"#,
    );
    let points = r#"[{"index": 0}, {"index": 1}]"#;
    assert_eq!(apply(input, points, 1, true), expected);
}

#[test]
fn the_point_control_marks_the_target() {
    let input = r#"[{"role": "user", "content": "a"}]"#;
    let expected = json(
        r#"[{"role": "user", "content": "a", "cache_control": {"type": "ephemeral", "ttl": "1h"}}]"#,
    );
    let points = r#"[{"role": "user", "control": {"type": "ephemeral", "ttl": "1h"}}]"#;
    assert_eq!(apply(input, points, 4, false), expected);
}

#[test]
fn a_null_control_means_the_ephemeral_default() {
    let input = r#"[{"role": "user", "content": "a"}]"#;
    let expected =
        json(r#"[{"role": "user", "content": "a", "cache_control": {"type": "ephemeral"}}]"#);
    let points = r#"[{"role": "user", "control": null}]"#;
    assert_eq!(apply(input, points, 4, false), expected);
}

#[test]
fn a_string_index_reaches_the_message_it_names() {
    let input = r#"[
        {"role": "user", "content": "a"},
        {"role": "user", "content": "b"}
    ]"#;
    let expected = json(
        r#"[
        {"role": "user", "content": "a"},
        {"role": "user", "content": "b", "cache_control": {"type": "ephemeral"}}
    ]"#,
    );
    assert_eq!(apply(input, r#"[{"index": "-1"}]"#, 4, false), expected);
}

#[test]
fn a_limit_warning_is_logged_once() {
    let (events, _captured) = capture_warnings();
    let result = apply(FOUR_MARKS, r#"[{"index": 0}, {"index": 1}]"#, 4, false);
    assert_eq!(result, json(FOUR_MARKS));
    let warnings = events.lock().unwrap().clone();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("cache breakpoint limit"));
}

#[test]
fn no_warning_is_logged_while_the_budget_lasts() {
    let (events, _captured) = capture_warnings();
    let input = r#"[{"role": "user", "content": "a"}]"#;
    let expected =
        json(r#"[{"role": "user", "content": "a", "cache_control": {"type": "ephemeral"}}]"#);
    assert_eq!(apply(input, r#"[{"role": "user"}]"#, 4, false), expected);
    assert!(events.lock().unwrap().is_empty());
}

#[test]
fn a_full_budget_warns_even_when_the_point_targets_nothing() {
    let (events, _captured) = capture_warnings();
    assert_eq!(
        apply(FOUR_MARKS, r#"[{"role": "system"}]"#, 4, false),
        json(FOUR_MARKS)
    );
    assert_eq!(events.lock().unwrap().len(), 1);
}
