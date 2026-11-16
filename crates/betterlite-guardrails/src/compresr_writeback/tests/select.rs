use rstest::rstest;

use super::support::{guardrail, messages};

const ALL_ROLES: &str = r#"[{"role":"system","content":"system prompt"},{"role":"user","content":"first user"},{"role":"tool","content":"tool output"},{"role":"function","content":"function output"},{"role":"assistant","content":"assistant text"},{"role":"user","content":"last user"}]"#;
const UNUSED_BASE: &str = "http://127.0.0.1:9";

#[rstest]
#[case::tool_outputs_by_default("", &[2, 3])]
#[case::tool_outputs_off("      compress_tool_outputs: false\n", &[])]
#[case::system_joins("      compress_system: true\n", &[0, 2, 3])]
#[case::history_joins("      compress_history: true\n", &[1, 2, 3])]
#[case::last_user_joins("      compress_last_user: true\n", &[2, 3, 5])]
#[case::everything(
    "      compress_system: true\n      compress_history: true\n      compress_last_user: true\n",
    &[0, 1, 2, 3, 5]
)]
fn flags_decide_select_targets(#[case] params: &str, #[case] expected: &[usize]) {
    let guardrail = guardrail(UNUSED_BASE, params);
    assert_eq!(
        guardrail.select_targets(&messages(ALL_ROLES), Some(5)),
        expected
    );
}

#[test]
fn history_and_last_user_together_select_targets() {
    let guardrail = guardrail(
        UNUSED_BASE,
        "      compress_history: true\n      compress_last_user: true\n",
    );
    let rows = messages(
        r#"[{"role":"user","content":"first user"},{"role":"user","content":"second user"}]"#,
    );
    assert_eq!(guardrail.select_targets(&rows, Some(1)), [0, 1]);
    assert_eq!(guardrail.select_targets(&rows, None), [0, 1]);
}

#[rstest]
#[case::one_short_of_the_minimum(r#""abcd""#, false)]
#[case::exactly_the_minimum(r#""abcde""#, true)]
#[case::counts_characters_not_bytes(r#""ééééé""#, true)]
#[case::four_multibyte_characters_are_short(r#""éééé""#, false)]
#[case::text_parts_join_with_a_blank_line(
    r#"[{"type":"text","text":"ab"},{"type":"text","text":"cd"}]"#,
    true
)]
#[case::text_parts_below_the_minimum(r#"[{"type":"text","text":"ab"}]"#, false)]
#[case::a_non_text_part_keeps_the_message(
    r#"[{"type":"text","text":"long enough text"},{"type":"image_url","image_url":{"url":"x"}}]"#,
    false
)]
#[case::empty_part_list(r#"[]"#, false)]
#[case::null_content("null", false)]
fn minimum_and_shape_decide_select_targets(#[case] content: &str, #[case] selected: bool) {
    let guardrail = guardrail(UNUSED_BASE, "");
    let rows = messages(&format!(r#"[{{"role":"tool","content":{content}}}]"#));
    assert_eq!(!guardrail.select_targets(&rows, None).is_empty(), selected);
}

#[test]
fn missing_content_is_not_selected_select_targets() {
    let guardrail = guardrail(UNUSED_BASE, "");
    let rows = messages(r#"[{"role":"tool"}]"#);
    assert!(guardrail.select_targets(&rows, None).is_empty());
}
