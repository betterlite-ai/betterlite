#![cfg(test)]

use rstest::rstest;

use super::to_ui_content;

#[rstest]
#[case::empty("", r#"{"kind":"text","text":""}"#)]
#[case::not_json("not json", r#"{"kind":"text","text":"not json"}"#)]
#[case::quoted(r#""quoted""#, r#"{"kind":"text","text":"quoted"}"#)]
#[case::number("1", r#"{"kind":"text","text":"1"}"#)]
#[case::empty_list("[]", r#"{"kind":"text","text":"[]"}"#)]
#[case::empty_object("{}", r#"{"kind":"fields","fields":[]}"#)]
#[case::empty_role_uses_type(
    r#"[{"role":"","type":"ai","content":"x","name":""}]"#,
    r#"{"kind":"messages","messages":[{"role":"assistant","content":"x"}]}"#
)]
#[case::unknown_type_is_not_a_role(r#"[{"type":"custom","tool_calls":[{"name":"a"}]}]"#, "")]
#[case::fields_keep_order(
    r#"{"b":{"y":1,"x":[1.5e20]},"a":2,"c":"s"}"#,
    r#"{"kind":"fields","fields":[{"key":"b","value":"{\"y\": 1, \"x\": [1.5e+20]}"},{"key":"a","value":"2"},{"key":"c","value":"s"}]}"#
)]
#[case::langchain_list(
    r#"[{"lc":1,"type":"constructor","kwargs":{"type":"human","content":"hi"}}]"#,
    r#"{"kind":"messages","messages":[{"role":"user","content":"hi"}]}"#
)]
#[case::langchain_unwraps_once(
    r#"{"lc":1,"type":"constructor","kwargs":{"type":"human","content":"hi","kwargs":{"role":"tool","content":"deep"}}}"#,
    r#"{"kind":"messages","messages":[{"role":"user","content":"hi"}]}"#
)]
#[case::function_arguments(
    r#"[{"role":"assistant","content":null,"tool_calls":[{"name":"outer","function":{"name":"","arguments":{"q":1}}}]}]"#,
    r#"{"kind":"messages","messages":[{"role":"assistant","content":"","tool_calls":[{"name":"outer","arguments":"{\"q\": 1}"}]}]}"#
)]
#[case::function_name(
    r#"[{"role":"assistant","tool_calls":[{"name":"outer","function":{"name":"inner"}}]}]"#,
    r#"{"kind":"messages","messages":[{"role":"assistant","content":"","tool_calls":[{"name":"inner","arguments":"{}"}]}]}"#
)]
#[case::call_arguments(
    r#"[{"role":"bot","tool_calls":[{"name":"t","args":null,"arguments":"raw"},{"name":"u","args":{"a":1},"arguments":"raw"},{"name":"v"},{"name":"w","arguments":[1]}]}]"#,
    r#"{"kind":"messages","messages":[{"role":"assistant","content":"","tool_calls":[{"name":"t","arguments":"raw"},{"name":"u","arguments":"{\"a\": 1}"},{"name":"v","arguments":"{}"},{"name":"w","arguments":"[1]"}]}]}"#
)]
#[case::unknown_role_is_user(
    r#"[{"role":"bot","content":"x"}]"#,
    r#"{"kind":"messages","messages":[{"role":"user","content":"x"}]}"#
)]
#[case::block_content(
    r#"[{"type":"ai","content":[{"type":"text","text":"a"},{"type":"reasoning"}],"name":"n"}]"#,
    r#"{"kind":"messages","messages":[{"role":"assistant","content":"a","name":"n"}]}"#
)]
#[case::numeric_role(
    r#"{"role":1,"content":"x"}"#,
    r#"{"kind":"fields","fields":[{"key":"role","value":"1"},{"key":"content","value":"x"}]}"#
)]
#[case::null_call_name(r#"[{"role":"user","content":"x","tool_calls":[{"name":null}]}]"#, "")]
#[case::text_function(
    r#"[{"role":"user","content":"x","tool_calls":[{"name":"a","function":"f"}]}]"#,
    ""
)]
#[case::object_calls(r#"[{"role":"user","content":"x","tool_calls":{"name":"a"}}]"#, "")]
#[case::null_function_name(
    r#"[{"role":"user","content":"x","tool_calls":[{"function":{"name":null}}]}]"#,
    ""
)]
#[case::numeric_call(r#"[{"role":"user","content":"x","tool_calls":[1]}]"#, "")]
#[case::no_content(r#"[{"role":"user"}]"#, "")]
#[case::type_without_content(r#"[{"type":"user"}]"#, "")]
#[case::not_an_object(r#"[{"role":"user","content":"x"},2]"#, "")]
#[case::numeric_kwargs(r#"[{"role":"user","content":"x","kwargs":3}]"#, "")]
#[case::kwargs_without_role(r#"[{"role":"user","content":"x","kwargs":{"content":"y"}}]"#, "")]
#[case::numeric_type(r#"[{"role":"user","content":"x","type":5}]"#, "")]
#[case::numeric_name(r#"[{"role":"user","content":"x","name":5}]"#, "")]
#[case::nulls_are_missing(
    r#"[{"role":"user","content":"x","tool_calls":null,"kwargs":null,"name":null}]"#,
    r#"{"kind":"messages","messages":[{"role":"user","content":"x"}]}"#
)]
#[case::system_and_tool(
    r#"[{"role":"system","content":"x"},{"role":"tool","content":"y","name":"t"}]"#,
    r#"{"kind":"messages","messages":[{"role":"system","content":"x"},{"role":"tool","content":"y","name":"t"}]}"#
)]
fn converts_like_litellm_ui_format(#[case] raw: &str, #[case] expected: &str) {
    // An empty expectation means LiteLLM shows the raw text unchanged
    let text = format!(
        r#"{{"kind":"text","text":{}}}"#,
        serde_json::to_string(raw).unwrap()
    );
    let wanted = if expected.is_empty() {
        text.as_str()
    } else {
        expected
    };
    assert_eq!(serde_json::to_string(&to_ui_content(raw)).unwrap(), wanted);
}
