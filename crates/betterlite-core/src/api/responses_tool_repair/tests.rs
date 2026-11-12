#![cfg(test)]

use indexmap::IndexMap;
use rstest::rstest;

use super::definition::{
    append_tool_call, create_tool_call_chunk, normalize_definition, reconstruct_from_tools,
};
use super::error::RepairError;
use super::value::Value;
use super::{call_exists, recover_call_id, repair_tool_results, tool_calls_of};

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn values(text: &str) -> Vec<Value> {
    serde_json::from_str(text).unwrap()
}

fn cached(text: &str) -> IndexMap<String, Value> {
    serde_json::from_str(text).unwrap()
}

fn repaired(messages: &str, tools: &str, cache: &str) -> Vec<Value> {
    repair_tool_results(&values(messages), &values(tools), &cached(cache)).unwrap()
}

const ASSISTANT_WITH_CALL: &str = r#"{"role":"assistant","tool_calls":[{"id":"call_9","type":"function","function":{"name":"f","arguments":"{}"}}]}"#;

#[rstest]
#[case::dict_call(r#"{"tool_calls":[{"id":"call_1"},{"id":"call_2"}]}"#, "call_1")]
#[case::object_call(r#"{"tool_calls":[{"__object__":{"attrs":{"id":"obj_1"}}}]}"#, "obj_1")]
#[case::object_assistant(r#"{"__object__":{"attrs":{"tool_calls":[{"id":"a"}]}}}"#, "a")]
#[case::numeric_id_becomes_text(r#"{"tool_calls":[{"id":7}]}"#, "7")]
#[case::null_id(r#"{"tool_calls":[{"id":null}]}"#, "")]
#[case::missing_id(r#"{"tool_calls":[{"name":"x"}]}"#, "")]
#[case::empty_list(r#"{"tool_calls":[]}"#, "")]
#[case::null_list(r#"{"tool_calls":null}"#, "")]
#[case::no_list(r#"{"role":"assistant"}"#, "")]
#[case::tuple_is_not_a_list(r#"{"tool_calls":{"__tuple__":[{"id":"a"}]}}"#, "")]
#[case::only_the_first_call_counts(r#"{"tool_calls":[{"id":""},{"id":"b"}]}"#, "")]
#[case::first_call_without_fields(r#"{"tool_calls":["text",{"id":"b"}]}"#, "")]
fn recovered_ids(#[case] assistant: &str, #[case] expected: &str) {
    assert_eq!(recover_call_id(&value(assistant)).unwrap(), expected);
}

#[test]
fn recovery_rejects_an_id_that_has_no_text_form() {
    let assistant = value(r#"{"tool_calls":[{"id":[1]}]}"#);
    assert_eq!(
        recover_call_id(&assistant),
        Err(RepairError::WrongType { field: "id" })
    );
}

#[rstest]
#[case::nearest_assistant(
    r#"[{"role":"user","content":"hi"},ASSISTANT,{"role":"tool","tool_call_id":"","content":"r"}]"#,
    r#"[{"role":"user","content":"hi"},ASSISTANT,{"role":"tool","tool_call_id":"call_9","content":"r"}]"#
)]
#[case::several_messages_back(
    r#"[ASSISTANT,{"role":"user","content":"hi"},{"role":"system","content":"s"},{"role":"tool","tool_call_id":"","content":"r"}]"#,
    r#"[ASSISTANT,{"role":"user","content":"hi"},{"role":"system","content":"s"},{"role":"tool","tool_call_id":"call_9","content":"r"}]"#
)]
#[case::null_id(
    r#"[ASSISTANT,{"role":"tool","tool_call_id":null,"content":"r"}]"#,
    r#"[ASSISTANT,{"role":"tool","tool_call_id":"call_9","content":"r"}]"#
)]
#[case::missing_id(
    r#"[ASSISTANT,{"role":"tool","content":"r"}]"#,
    r#"[ASSISTANT,{"role":"tool","content":"r","tool_call_id":"call_9"}]"#
)]
#[case::only_the_nearest_assistant_is_asked(
    r#"[ASSISTANT,{"role":"assistant","content":"plain"},{"role":"tool","tool_call_id":"","content":"r"}]"#,
    r#"[ASSISTANT,{"role":"assistant","content":"plain"}]"#
)]
#[case::object_message_with_the_attribute(
    r#"[ASSISTANT,{"__object__":{"attrs":{"role":"tool","tool_call_id":""},"get":{"mapping":{"role":"tool"}}}}]"#,
    r#"[ASSISTANT,{"__object__":{"attrs":{"role":"tool","tool_call_id":"call_9"},"get":{"mapping":{"role":"tool"}}}}]"#
)]
#[case::object_message_without_the_attribute(
    r#"[ASSISTANT,{"__object__":{"attrs":{"role":"tool"},"get":{"mapping":{"role":"tool"}}}}]"#,
    r#"[ASSISTANT,{"__object__":{"attrs":{"role":"tool"},"get":{"mapping":{"role":"tool"}}}}]"#
)]
#[case::object_assistant(
    r#"[{"__object__":{"attrs":{"role":"assistant","tool_calls":[{"id":"c1"}]},"get":{"mapping":{"role":"assistant"}}}},{"role":"tool","tool_call_id":"","content":"r"}]"#,
    r#"[{"__object__":{"attrs":{"role":"assistant","tool_calls":[{"id":"c1"}]},"get":{"mapping":{"role":"assistant"}}}},{"role":"tool","tool_call_id":"c1","content":"r"}]"#
)]
fn empty_ids_are_recovered(#[case] messages: &str, #[case] expected: &str) {
    let assistant = ASSISTANT_WITH_CALL;
    let input = messages.replace("ASSISTANT", assistant);
    assert_eq!(
        repaired(&input, "[]", "{}"),
        values(&expected.replace("ASSISTANT", assistant))
    );
}

const USER: &str = r#"{"role":"user","content":"hi"}"#;
const CALL_1: &str =
    r#"{"id":"call_1","type":"function","function":{"name":"weather","arguments":{"x":1}}}"#;

#[rstest]
#[case::cache_hit_builds_the_chunk(
    r#"[USER,{"role":"assistant","content":null},{"role":"tool","tool_call_id":"call_1","content":"r"}]"#,
    "[]",
    r#"{"call_1":CALL_1}"#,
    r#"[USER,{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"weather","arguments":"{\"x\": 1}"},"index":0}]},{"role":"tool","tool_call_id":"call_1","content":"r"}]"#
)]
#[case::text_arguments_pass_through(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    "[]",
    r#"{"call_1":{"id":"call_1","function":{"name":"n","arguments":"{\"a\":2}"}}}"#,
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"n","arguments":"{\"a\":2}"},"index":0}]},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::index_is_the_call_count(
    r#"[{"role":"assistant","tool_calls":[{"id":"call_0"},{"id":"call_5"}]},{"role":"tool","tool_call_id":"call_1"}]"#,
    "[]",
    r#"{"call_1":CALL_1}"#,
    r#"[{"role":"assistant","tool_calls":[{"id":"call_0"},{"id":"call_5"},{"id":"call_1","type":"function","function":{"name":"weather","arguments":"{\"x\": 1}"},"index":2}]},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::present_call_is_not_duplicated(
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1"}]},{"role":"tool","tool_call_id":"call_1"}]"#,
    r#"[{"name":"search"}]"#,
    r#"{"call_1":CALL_1}"#,
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1"}]},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::numeric_call_id_is_not_a_match(
    r#"[{"role":"assistant","tool_calls":[{"id":7}]},{"role":"tool","tool_call_id":"7"}]"#,
    r#"[{"name":"search"}]"#,
    "{}",
    r#"[{"role":"assistant","tool_calls":[{"id":7},{"id":"7","type":"function","function":{"name":"search","arguments":"{}"},"index":1}]},{"role":"tool","tool_call_id":"7"}]"#
)]
#[case::tools_rebuild_a_missing_call(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    r#"[{"type":"function","function":{"name":"lookup"}}]"#,
    "{}",
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"},"index":0}]},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::first_named_tool_wins(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    r#"[{"type":"web_search"},"text",{"name":"first"},{"name":"second"}]"#,
    "{}",
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"first","arguments":"{}"},"index":0}]},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::empty_cache_entry_uses_the_tools(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    r#"[{"name":"search"}]"#,
    r#"{"call_1":{}}"#,
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{}"},"index":0}]},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::no_cache_and_no_named_tool(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    r#"[{"type":"web_search"}]"#,
    "{}",
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::no_assistant_before_the_result(
    r#"[USER,{"role":"tool","tool_call_id":"call_1"}]"#,
    "[]",
    r#"{"call_1":CALL_1}"#,
    r#"[USER,{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::an_assistant_after_the_result_is_not_used(
    r#"[USER,{"role":"tool","tool_call_id":"call_1"},{"role":"assistant"}]"#,
    "[]",
    r#"{"call_1":CALL_1}"#,
    r#"[USER,{"role":"tool","tool_call_id":"call_1"},{"role":"assistant"}]"#
)]
#[case::two_results_one_call(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"},{"role":"tool","tool_call_id":"call_1"}]"#,
    "[]",
    r#"{"call_1":CALL_1}"#,
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"weather","arguments":"{\"x\": 1}"},"index":0}]},{"role":"tool","tool_call_id":"call_1"},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::two_missing_calls_get_consecutive_indexes(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"},{"role":"tool","tool_call_id":"call_2"}]"#,
    "[]",
    r#"{"call_1":CALL_1,"call_2":{"id":"call_2","function":{"name":"other"}}}"#,
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"weather","arguments":"{\"x\": 1}"},"index":0},{"id":"call_2","type":"function","function":{"name":"other","arguments":"{}"},"index":1}]},{"role":"tool","tool_call_id":"call_1"},{"role":"tool","tool_call_id":"call_2"}]"#
)]
#[case::null_list_stays_null_on_a_dict(
    r#"[{"role":"assistant","tool_calls":null},{"role":"tool","tool_call_id":"call_1"}]"#,
    "[]",
    r#"{"call_1":CALL_1}"#,
    r#"[{"role":"assistant","tool_calls":null},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::null_list_becomes_a_list_on_an_object(
    r#"[{"__object__":{"attrs":{"role":"assistant","tool_calls":null},"get":{"mapping":{"role":"assistant"}}}},{"role":"tool","tool_call_id":"call_1"}]"#,
    "[]",
    r#"{"call_1":CALL_1}"#,
    r#"[{"__object__":{"attrs":{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"weather","arguments":"{\"x\": 1}"},"index":0}]},"get":{"mapping":{"role":"assistant"}}}},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::object_without_the_attribute_is_untouched(
    r#"[{"__object__":{"attrs":{"role":"assistant"},"get":{"mapping":{"role":"assistant"}}}},{"role":"tool","tool_call_id":"call_1"}]"#,
    "[]",
    r#"{"call_1":CALL_1}"#,
    r#"[{"__object__":{"attrs":{"role":"assistant"},"get":{"mapping":{"role":"assistant"}}}},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::cached_object_definition(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    "[]",
    r#"{"call_1":{"__object__":{"attrs":{"id":"call_1","type":"function","function":{"__object__":{"attrs":{"name":"obj_fn","arguments":"{\"k\":1}"}}}}}}}"#,
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"obj_fn","arguments":"{\"k\":1}"},"index":0}]},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::cached_bytes_are_a_definition(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    r#"[{"name":"search"}]"#,
    r#"{"call_1":{"__bytes__":"x"}}"#,
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::empty_cached_bytes_use_the_tools(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    r#"[{"name":"search"}]"#,
    r#"{"call_1":{"__bytes__":""}}"#,
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{}"},"index":0}]},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::cached_list_is_a_definition(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    r#"[{"name":"search"}]"#,
    r#"{"call_1":[1]}"#,
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::empty_cached_list_uses_the_tools(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    r#"[{"name":"search"}]"#,
    r#"{"call_1":[]}"#,
    r#"[{"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{}"},"index":0}]},{"role":"tool","tool_call_id":"call_1"}]"#
)]
#[case::cached_text_is_not_a_definition(
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#,
    "[]",
    r#"{"call_1":"text"}"#,
    r#"[{"role":"assistant"},{"role":"tool","tool_call_id":"call_1"}]"#
)]
fn missing_calls_are_restored(
    #[case] messages: &str,
    #[case] tools: &str,
    #[case] cache: &str,
    #[case] expected: &str,
) {
    let fill = |text: &str| text.replace("USER", USER).replace("CALL_1", CALL_1);
    assert_eq!(
        repaired(&fill(messages), tools, &fill(cache)),
        values(&fill(expected))
    );
}

#[rstest]
#[case::tool_without_a_result_id(
    r#"[{"role":"user","content":"hi"},{"role":"tool","tool_call_id":""}]"#,
    r#"[{"role":"user","content":"hi"}]"#
)]
#[case::null_id_is_dropped(
    r#"[{"role":"user"},{"role":"tool","tool_call_id":null}]"#,
    r#"[{"role":"user"}]"#
)]
#[case::missing_id_is_dropped(r#"[{"role":"user"},{"role":"tool"}]"#, r#"[{"role":"user"}]"#)]
#[case::only_tool_messages_are_kept(
    r#"[{"role":"tool","tool_call_id":""}]"#,
    r#"[{"role":"tool","tool_call_id":""}]"#
)]
#[case::every_tool_message_is_kept_when_all_are_tools(
    r#"[{"role":"tool","tool_call_id":""},{"role":"tool","tool_call_id":null},{"role":"tool"}]"#,
    r#"[{"role":"tool","tool_call_id":""},{"role":"tool","tool_call_id":null},{"role":"tool"}]"#
)]
#[case::empty_history("[]", "[]")]
#[case::zero_is_a_usable_id(
    r#"[{"role":"user"},{"role":"tool","tool_call_id":0}]"#,
    r#"[{"role":"user"},{"role":"tool","tool_call_id":0}]"#
)]
#[case::every_dropped_index_is_removed(
    r#"[{"role":"tool","tool_call_id":""},{"role":"user","content":"a"},{"role":"tool","tool_call_id":""},{"role":"user","content":"b"},{"role":"tool","tool_call_id":""}]"#,
    r#"[{"role":"user","content":"a"},{"role":"user","content":"b"}]"#
)]
#[case::later_results_survive_a_drop(
    r#"[{"role":"user"},{"role":"tool","tool_call_id":""},{"role":"assistant","tool_calls":[{"id":"a"}]},{"role":"tool","tool_call_id":"a"}]"#,
    r#"[{"role":"user"},{"role":"assistant","tool_calls":[{"id":"a"}]},{"role":"tool","tool_call_id":"a"}]"#
)]
#[case::an_id_without_an_assistant_is_kept(
    r#"[{"role":"user"},{"role":"tool","tool_call_id":"x"}]"#,
    r#"[{"role":"user"},{"role":"tool","tool_call_id":"x"}]"#
)]
fn unmatched_results(#[case] messages: &str, #[case] expected: &str) {
    assert_eq!(repaired(messages, "[]", "{}"), values(expected));
}

#[rstest]
#[case::text(r#"["text"]"#, RepairError::NoGetMethod)]
#[case::number("[3]", RepairError::NoGetMethod)]
#[case::null("[null]", RepairError::NoGetMethod)]
#[case::list("[[1]]", RepairError::NoGetMethod)]
#[case::object_without_get(
    r#"[{"__object__":{"attrs":{"role":"user"}}}]"#,
    RepairError::NoGetMethod
)]
#[case::get_raises_type_error(
    r#"[{"__object__":{"attrs":{"role":"user"},"get":{"raises":"type_error"}}}]"#,
    RepairError::GetterFailed
)]
#[case::get_raises_attribute_error(
    r#"[{"__object__":{"attrs":{"role":"user"},"get":{"raises":"attribute_error"}}}]"#,
    RepairError::GetterFailed
)]
#[case::get_raises_anything_else(
    r#"[{"__object__":{"attrs":{"role":"user"},"get":{"raises":"other"}}}]"#,
    RepairError::GetterFailed
)]
fn messages_must_have_a_working_get(#[case] messages: &str, #[case] expected: RepairError) {
    let result = repair_tool_results(&values(messages), &[], &cached("{}"));
    assert_eq!(result, Err(expected));
}

#[test]
fn the_input_messages_are_not_changed() {
    let messages = values(&format!(
        r#"[{USER},{{"role":"assistant"}},{{"role":"tool","tool_call_id":"call_1"}},{{"role":"tool","tool_call_id":""}}]"#
    ));
    let before = messages.clone();
    let cache = cached(&format!(r#"{{"call_1":{CALL_1}}}"#));
    let fixed = repair_tool_results(&messages, &[], &cache).unwrap();
    assert_eq!(messages, before);
    assert_ne!(fixed, before);
}

#[rstest]
#[case::dict_function_name(r#"[{"type":"function","function":{"name":"inner"}}]"#, Some("inner"))]
#[case::top_level_name(r#"[{"name":"top"}]"#, Some("top"))]
#[case::function_name_wins(r#"[{"name":"top","function":{"name":"inner"}}]"#, Some("inner"))]
#[case::empty_function_name_falls_back(r#"[{"name":"top","function":{"name":""}}]"#, Some("top"))]
#[case::null_function(r#"[{"name":"top","function":null}]"#, Some("top"))]
#[case::empty_function(r#"[{"name":"top","function":{}}]"#, Some("top"))]
#[case::non_dict_tools_are_skipped(r#"["text",3,null,{"name":"x"}]"#, Some("x"))]
#[case::unnamed_tools(
    r#"[{"type":"web_search"},{"name":""},{"function":{"name":null}}]"#,
    None
)]
#[case::no_tools("[]", None)]
fn reconstructed_definitions(#[case] tools: &str, #[case] name: Option<&str>) {
    let expected = name.map(|found| {
        value(&format!(
            r#"{{"id":"call_1","type":"function","function":{{"name":"{found}","arguments":"{{}}"}}}}"#
        ))
    });
    assert_eq!(
        reconstruct_from_tools("call_1", &values(tools)),
        Ok(expected)
    );
}

#[test]
fn a_numeric_tool_name_is_kept_as_it_is() {
    let found = reconstruct_from_tools("c", &values(r#"[{"name":7}]"#)).unwrap();
    assert_eq!(
        found,
        Some(value(
            r#"{"id":"c","type":"function","function":{"name":7,"arguments":"{}"}}"#
        ))
    );
}

#[test]
fn a_function_that_is_not_a_mapping_is_an_error() {
    let tools = values(r#"[{"function":"abc","name":"top"}]"#);
    assert_eq!(
        reconstruct_from_tools("c", &tools),
        Err(RepairError::WrongType { field: "function" })
    );
}

#[rstest]
#[case::null("null")]
#[case::empty_dict("{}")]
#[case::empty_text(r#""""#)]
#[case::empty_list("[]")]
#[case::zero("0")]
#[case::false_flag("false")]
fn falsy_definitions_do_not_normalize(#[case] definition: &str) {
    assert_eq!(normalize_definition(&value(definition), "c"), Ok(None));
}

#[rstest]
#[case::dict_gets_id_and_type(
    r#"{"function":{"name":"f"}}"#,
    r#"{"function":{"name":"f"},"id":"c","type":"function"}"#
)]
#[case::explicit_id_and_type_stay(r#"{"id":"x","type":"custom"}"#, r#"{"id":"x","type":"custom"}"#)]
#[case::empty_id_and_type_are_replaced(
    r#"{"id":"","type":null}"#,
    r#"{"id":"c","type":"function"}"#
)]
#[case::object_fields_become_a_dict(
    r#"{"__object__":{"attrs":{"id":"i","type":"t","function":{"name":"n"}}}}"#,
    r#"{"id":"i","type":"t","function":{"name":"n"}}"#
)]
#[case::object_with_a_getter(
    r#"{"__object__":{"attrs":{"id":"attr"},"get":{"mapping":{"id":"mapped","type":"t"}}}}"#,
    r#"{"id":"mapped","type":"t","function":null}"#
)]
#[case::object_with_only_a_type(
    r#"{"__object__":{"attrs":{"type":"t"}}}"#,
    r#"{"id":"c","type":"t","function":null}"#
)]
#[case::object_with_only_a_function(
    r#"{"__object__":{"attrs":{"function":{"name":"n"}}}}"#,
    r#"{"id":"c","type":"function","function":{"name":"n"}}"#
)]
#[case::object_with_only_an_id(
    r#"{"__object__":{"attrs":{"id":"i"}}}"#,
    r#"{"id":"i","type":"function","function":null}"#
)]
#[case::function_object_becomes_a_dict(
    r#"{"id":"i","function":{"__object__":{"attrs":{"name":"n","arguments":"a"}}}}"#,
    r#"{"id":"i","type":"function","function":{"name":"n","arguments":"a"}}"#
)]
#[case::function_object_with_only_a_name(
    r#"{"id":"i","function":{"__object__":{"attrs":{"name":"n"}}}}"#,
    r#"{"id":"i","type":"function","function":{"name":"n","arguments":null}}"#
)]
#[case::function_object_without_fields_stays(
    r#"{"id":"i","function":{"__object__":{"attrs":{"other":1}}}}"#,
    r#"{"id":"i","type":"function","function":{"__object__":{"attrs":{"other":1}}}}"#
)]
#[case::function_text_stays(
    r#"{"id":"i","function":"abc"}"#,
    r#"{"id":"i","type":"function","function":"abc"}"#
)]
#[case::function_null_stays(
    r#"{"id":"i","function":null}"#,
    r#"{"id":"i","type":"function","function":null}"#
)]
fn normalized_definitions(#[case] definition: &str, #[case] expected: &str) {
    assert_eq!(
        normalize_definition(&value(definition), "c"),
        Ok(Some(value(expected)))
    );
}

#[test]
fn an_object_that_exposes_no_tool_call_fields_does_not_normalize() {
    let definition = value(r#"{"__object__":{"attrs":{"other":1}}}"#);
    assert_eq!(normalize_definition(&definition, "c"), Ok(None));
}

#[test]
fn an_object_getter_failure_surfaces_while_normalizing() {
    let definition = value(r#"{"__object__":{"attrs":{"id":"i"},"get":{"raises":"other"}}}"#);
    assert_eq!(
        normalize_definition(&definition, "c"),
        Err(RepairError::GetterFailed)
    );
}

#[rstest]
#[case::empty_dict(
    "{}",
    0,
    r#"{"id":"c","type":"function","function":{"name":"","arguments":"{}"},"index":0}"#
)]
#[case::index_is_kept(
    "{}",
    2,
    r#"{"id":"c","type":"function","function":{"name":"","arguments":"{}"},"index":2}"#
)]
#[case::explicit_values(
    r#"{"id":"x","type":"custom","function":{"name":"n","arguments":"{\"a\":1}"}}"#,
    1,
    r#"{"id":"x","type":"custom","function":{"name":"n","arguments":"{\"a\":1}"},"index":1}"#
)]
#[case::object_arguments_use_python_json_text(
    r#"{"function":{"name":"n","arguments":{"a":1,"b":[1,2]}}}"#,
    0,
    r#"{"id":"c","type":"function","function":{"name":"n","arguments":"{\"a\": 1, \"b\": [1, 2]}"},"index":0}"#
)]
#[case::empty_id_is_kept(
    r#"{"id":""}"#,
    0,
    r#"{"id":"","type":"function","function":{"name":"","arguments":"{}"},"index":0}"#
)]
#[case::null_id_falls_back(
    r#"{"id":null,"type":null}"#,
    0,
    r#"{"id":"c","type":"function","function":{"name":"","arguments":"{}"},"index":0}"#
)]
#[case::empty_type_is_kept(
    r#"{"type":""}"#,
    0,
    r#"{"id":"c","type":"","function":{"name":"","arguments":"{}"},"index":0}"#
)]
#[case::numeric_id_and_name_become_text(
    r#"{"id":7,"function":{"name":8}}"#,
    0,
    r#"{"id":"7","type":"function","function":{"name":"8","arguments":"{}"},"index":0}"#
)]
#[case::falsy_arguments_default(
    r#"{"function":{"name":"n","arguments":{}}}"#,
    0,
    r#"{"id":"c","type":"function","function":{"name":"n","arguments":"{}"},"index":0}"#
)]
#[case::zero_arguments_default(
    r#"{"function":{"name":"n","arguments":0}}"#,
    0,
    r#"{"id":"c","type":"function","function":{"name":"n","arguments":"{}"},"index":0}"#
)]
#[case::object_function(
    r#"{"function":{"__object__":{"attrs":{"name":"n","arguments":"a"}}}}"#,
    0,
    r#"{"id":"c","type":"function","function":{"name":"n","arguments":"a"},"index":0}"#
)]
#[case::null_function(
    r#"{"function":null}"#,
    0,
    r#"{"id":"c","type":"function","function":{"name":"","arguments":"{}"},"index":0}"#
)]
fn tool_call_chunks(#[case] definition: &str, #[case] index: usize, #[case] expected: &str) {
    assert_eq!(
        create_tool_call_chunk(&value(definition), "c", index),
        Ok(value(expected))
    );
}

#[rstest]
#[case::list_name(r#"{"function":{"name":[1]}}"#, "name")]
#[case::dict_id(r#"{"id":{"a":1}}"#, "id")]
#[case::list_type(r#"{"type":[1]}"#, "type")]
#[case::object_arguments(
    r#"{"function":{"name":"n","arguments":{"__object__":{"attrs":{}}}}}"#,
    "arguments"
)]
fn chunk_fields_without_a_text_form_are_errors(#[case] definition: &str, #[case] field: &str) {
    let result = create_tool_call_chunk(&value(definition), "c", 0);
    assert!(matches!(result, Err(RepairError::WrongType { field: found }) if found == field));
}

#[rstest]
#[case::dict_value(r#"{"a":1}"#, "a", Ok(Some("1")))]
#[case::missing_key(r#"{"a":1}"#, "b", Ok(None))]
#[case::null_value(r#"{"a":null}"#, "a", Ok(None))]
#[case::object_attribute(r#"{"__object__":{"attrs":{"a":"x"}}}"#, "a", Ok(Some(r#""x""#)))]
#[case::getter_wins_over_the_attribute(
    r#"{"__object__":{"attrs":{"a":"attr"},"get":{"mapping":{"a":"mapped"}}}}"#,
    "a",
    Ok(Some(r#""mapped""#))
)]
#[case::getter_without_the_key_does_not_read_the_attribute(
    r#"{"__object__":{"attrs":{"a":"attr"},"get":{"mapping":{}}}}"#,
    "a",
    Ok(None)
)]
#[case::type_error_falls_back_to_the_attribute(
    r#"{"__object__":{"attrs":{"a":"attr"},"get":{"raises":"type_error"}}}"#,
    "a",
    Ok(Some(r#""attr""#))
)]
#[case::attribute_error_falls_back_to_the_attribute(
    r#"{"__object__":{"attrs":{"a":"attr"},"get":{"raises":"attribute_error"}}}"#,
    "a",
    Ok(Some(r#""attr""#))
)]
#[case::other_errors_propagate(
    r#"{"__object__":{"attrs":{"a":"attr"},"get":{"raises":"other"}}}"#,
    "a",
    Err(RepairError::GetterFailed)
)]
#[case::null_target("null", "a", Ok(None))]
#[case::text_target(r#""abc""#, "a", Ok(None))]
#[case::number_target("5", "a", Ok(None))]
#[case::list_target("[1]", "a", Ok(None))]
fn mapping_or_attribute_values(
    #[case] target: &str,
    #[case] key: &str,
    #[case] expected: Result<Option<&str>, RepairError>,
) {
    let found = value(target).field(key).map(Option::<&Value>::cloned);
    assert_eq!(found, expected.map(|item| item.map(value)));
}

#[rstest]
#[case::list(r#"{"tool_calls":[{"id":"a"},"b"]}"#, r#"[{"id":"a"},"b"]"#)]
#[case::tuple(r#"{"tool_calls":{"__tuple__":[{"id":"a"}]}}"#, r#"[{"id":"a"}]"#)]
#[case::dict_iterates_its_keys(r#"{"tool_calls":{"a":1,"b":2}}"#, r#"["a","b"]"#)]
#[case::null(r#"{"tool_calls":null}"#, "[]")]
#[case::missing(r#"{"role":"assistant"}"#, "[]")]
#[case::text(r#"{"tool_calls":"abc"}"#, "[]")]
#[case::bytes(r#"{"tool_calls":{"__bytes__":"abc"}}"#, "[]")]
#[case::number(r#"{"tool_calls":3}"#, "[]")]
#[case::object_attribute(
    r#"{"__object__":{"attrs":{"tool_calls":[{"id":"a"}]}}}"#,
    r#"[{"id":"a"}]"#
)]
#[case::object_value_is_not_iterable(r#"{"tool_calls":{"__object__":{"attrs":{"id":"a"}}}}"#, "[]")]
fn tool_call_lists(#[case] assistant: &str, #[case] expected: &str) {
    assert_eq!(tool_calls_of(&value(assistant)), values(expected));
}

#[rstest]
#[case::match_by_text(r#"[{"id":"a"},{"id":"b"}]"#, "b", true)]
#[case::no_match(r#"[{"id":"a"}]"#, "b", false)]
#[case::numeric_id_is_not_text(r#"[{"id":7}]"#, "7", false)]
#[case::object_id(r#"[{"__object__":{"attrs":{"id":"a"}}}]"#, "a", true)]
#[case::dict_without_id(r#"[{"name":"a"}]"#, "a", false)]
#[case::null_id(r#"[{"id":null}]"#, "", false)]
#[case::text_items_have_no_id(r#"["a"]"#, "a", false)]
#[case::no_calls("[]", "a", false)]
fn existing_calls(#[case] calls: &str, #[case] id: &str, #[case] expected: bool) {
    assert_eq!(call_exists(&values(calls), id), expected);
}

#[rstest]
#[case::dict_gains_the_list(r#"{"role":"a"}"#, r#"{"role":"a","tool_calls":[{"id":"c"}]}"#)]
#[case::dict_list_grows(
    r#"{"tool_calls":[{"id":"x"}]}"#,
    r#"{"tool_calls":[{"id":"x"},{"id":"c"}]}"#
)]
#[case::dict_null_stays_null(r#"{"tool_calls":null}"#, r#"{"tool_calls":null}"#)]
#[case::dict_text_is_untouched(r#"{"tool_calls":"abc"}"#, r#"{"tool_calls":"abc"}"#)]
#[case::object_null_becomes_a_list(
    r#"{"__object__":{"attrs":{"tool_calls":null}}}"#,
    r#"{"__object__":{"attrs":{"tool_calls":[{"id":"c"}]}}}"#
)]
#[case::object_list_grows(
    r#"{"__object__":{"attrs":{"tool_calls":[{"id":"x"}]}}}"#,
    r#"{"__object__":{"attrs":{"tool_calls":[{"id":"x"},{"id":"c"}]}}}"#
)]
#[case::object_without_the_attribute(
    r#"{"__object__":{"attrs":{"role":"a"}}}"#,
    r#"{"__object__":{"attrs":{"role":"a"}}}"#
)]
#[case::object_text_is_untouched(
    r#"{"__object__":{"attrs":{"tool_calls":"abc"}}}"#,
    r#"{"__object__":{"attrs":{"tool_calls":"abc"}}}"#
)]
#[case::text_assistant(r#""abc""#, r#""abc""#)]
#[case::list_assistant("[1]", "[1]")]
fn appended_calls(#[case] assistant: &str, #[case] expected: &str) {
    let mut message = value(assistant);
    append_tool_call(&mut message, value(r#"{"id":"c"}"#));
    assert_eq!(message, value(expected));
}
