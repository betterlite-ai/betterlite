#![cfg(test)]

mod cases;
mod support;

use betterlite_openai::output_limits::RejectedFieldsError;
use rstest::rstest;

use super::{
    EXTRA_INPUTS_MARKER, FoundryRepair, INVALID_TOOL_CALL_INDEX_MARKER, RepairError,
    extra_parameters, first_bracket_group, repair, tool_level_field_names,
};

// One row per foundry-rejected-fields oracle case file, each driving the production repair.
#[rstest]
#[case::drop_top_level_loc(cases::drop_top_level_loc)]
#[case::drop_tool_level_extra_fields(cases::drop_tool_level_extra_fields)]
#[case::remove_tool_call_index(cases::remove_tool_call_index)]
#[case::extra_parameters_list(cases::extra_parameters_list)]
#[case::repair_without_drop_policy_for_tools(cases::repair_without_drop_policy_for_tools)]
#[case::unrelated_error_no_retry(cases::unrelated_error_no_retry)]
#[case::two_repairs_only(cases::two_repairs_only)]
#[case::repair_trigger_literals(cases::repair_trigger_literals)]
fn rejected_fields_cases(#[case] check: fn()) {
    check();
}

#[test]
fn extra_parameters_strip_whitespace_then_single_then_double_quotes() {
    assert_eq!(
        extra_parameters("parameters [ 'a' , \"b\" ,, \"\" ] remain"),
        vec![String::from("a"), String::from("b")]
    );
    // the single quotes come off before the double quotes, so a name wrapped in both keeps its single quotes
    assert_eq!(
        extra_parameters("parameters [\"'c'\"] remain"),
        vec![String::from("'c'")]
    );
    // a backslash left by the JSON escape keeps the surrounding double quote in the name
    assert_eq!(
        extra_parameters("parameters [\\\"d\\\"] remain"),
        vec![String::from("\\\"d\\")]
    );
    // Python's strip also removes the four separators below the White_Space property
    assert_eq!(
        extra_parameters("parameters [\u{1c}'a'\u{1f}] remain"),
        vec![String::from("a")]
    );
}

#[test]
fn first_bracket_group_skips_a_bracket_open_at_the_end_of_its_line() {
    assert_eq!(first_bracket_group("a [b] c [d]"), Some("b"));
    assert_eq!(first_bracket_group("a [b\nc] d [e]"), Some("e"));
    assert_eq!(first_bracket_group("a [b\n]"), None);
    assert_eq!(first_bracket_group("[]"), Some(""));
    assert_eq!(first_bracket_group("no brackets"), None);
}

#[test]
fn tool_level_fields_are_read_left_to_right() {
    assert_eq!(
        tool_level_field_names("tools[0].a-tools[1].b"),
        vec![String::from("a-tools")]
    );
    assert_eq!(
        tool_level_field_names("tools[0].extra.more"),
        vec![String::from("extra")]
    );
    // the same field named twice comes out once
    assert_eq!(
        tool_level_field_names("tools[0].extra tools[1].extra"),
        vec![String::from("extra")]
    );
    assert!(tool_level_field_names("tools[0].").is_empty());
    assert_eq!(
        tool_level_field_names("tools[\u{663}].extra"),
        vec![String::from("extra")]
    );
    assert_eq!(
        tool_level_field_names("tools[0].v2\u{e9}_9-x"),
        vec![String::from("v2\u{e9}_9-x")]
    );
    // the word class keeps a letter-number like \u{2163} and stops at a combining mark
    assert_eq!(
        tool_level_field_names("tools[0].a\u{2163}b tools[1].a\u{301}b tools[2].a\u{345}b"),
        vec![String::from("a\u{2163}b"), String::from("a")]
    );
}

#[test]
fn tool_level_markers_need_a_decimal_index_and_a_dot() {
    let policy = FoundryRepair {
        call: false,
        global: false,
    };
    assert!(policy.should_retry(&format!("{EXTRA_INPUTS_MARKER} tools[0].")));
    assert!(policy.should_retry(&format!("{EXTRA_INPUTS_MARKER} extra tools[12].x")));
    assert!(policy.should_retry(&format!("{EXTRA_INPUTS_MARKER} tools[\u{663}].")));
    // \d is the decimal-digit category, so a superscript two is not an index
    assert!(!policy.should_retry(&format!("{EXTRA_INPUTS_MARKER} tools[\u{b2}].")));
    // the pattern is not anchored, so a prefix does not hide it
    assert!(policy.should_retry(&format!("{EXTRA_INPUTS_MARKER} xtools[0].")));
    assert!(!policy.should_retry(&format!("{EXTRA_INPUTS_MARKER} tools[].")));
    assert!(!policy.should_retry(&format!("{EXTRA_INPUTS_MARKER} tools[x].")));
    assert!(!policy.should_retry(&format!("{EXTRA_INPUTS_MARKER} tools[0]")));
}

#[test]
fn messages_that_are_not_a_list_fail_the_repair() {
    let error = support::wrapped(INVALID_TOOL_CALL_INDEX_MARKER);
    let not_an_array = repair(
        &error,
        &support::body(r#"{"model":"deployment","messages":"hi"}"#),
    );
    assert!(matches!(not_an_array, Err(RepairError::Messages { .. })));
    let not_an_object = repair(
        &error,
        &support::body(r#"{"model":"deployment","messages":["hi"]}"#),
    );
    assert!(matches!(not_an_object, Err(RepairError::Messages { .. })));
}

#[test]
fn an_error_that_is_not_json_fails_the_repair() {
    let result = repair("not json", &support::body(r#"{"model":"deployment"}"#));
    assert!(matches!(
        result,
        Err(RepairError::Rejected(RejectedFieldsError::Json { .. }))
    ));
}

#[test]
fn a_json_error_that_is_not_an_object_fails_the_repair() {
    let result = repair("[1, 2]", &support::body(r#"{"model":"deployment"}"#));
    assert!(matches!(
        result,
        Err(RepairError::Rejected(
            RejectedFieldsError::ResponseNotObject
        ))
    ));
}

#[test]
fn tool_calls_that_are_not_a_list_are_left_alone() {
    let request = support::body(
        r#"{"model":"deployment","messages":[{"role":"assistant","tool_calls":{"index":0}}]}"#,
    );
    let repaired = repair(&support::wrapped(INVALID_TOOL_CALL_INDEX_MARKER), &request)
        .expect("the repair runs");
    assert_eq!(repaired, request);
}

#[test]
fn a_tool_call_that_is_not_an_object_keeps_its_place() {
    let request = support::body(
        r#"{"model":"deployment","messages":[{"role":"assistant","tool_calls":[{"index":0},"plain"]}]}"#,
    );
    let repaired = repair(&support::wrapped(INVALID_TOOL_CALL_INDEX_MARKER), &request)
        .expect("the repair runs");
    assert_eq!(
        repaired,
        support::body(
            r#"{"model":"deployment","messages":[{"role":"assistant","tool_calls":[{}, "plain"]}]}"#,
        )
    );
}

#[test]
fn tool_level_removal_skips_a_tools_value_that_is_not_a_list() {
    let request = support::body(r#"{"model":"deployment","tools":{"extra":1}}"#);
    let repaired = repair(
        &support::wrapped("Extra inputs are not permitted tools[0].extra"),
        &request,
    )
    .expect("the repair runs");
    assert_eq!(repaired, request);
}
