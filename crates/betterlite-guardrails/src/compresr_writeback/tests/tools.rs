use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::support::{
    TOOL_CONVERSATION, apply_as_caller, conversation, field, guardrail, reply, serve,
};
use crate::compresr_recovery::tools::merged_tools;
use crate::generic_guardrail_contracts::GuardrailToolParam;

const SEARCH: &str = r#"{"type":"function","function":{"name":"search"}}"#;
const RETRIEVE: &str = r#"{"type":"function","function":{"name":"compresr_retrieve"}}"#;

fn function_of(entry: &GuardrailToolParam) -> GuardrailValue {
    entry.extra.get("function").cloned().unwrap()
}

fn names(tools: &[GuardrailToolParam]) -> Vec<String> {
    tools
        .iter()
        .map(|entry| {
            let GuardrailValue::Text(name) = field(&function_of(entry), "name") else {
                panic!("a function tool is named by text");
            };
            name
        })
        .collect()
}

#[tokio::test]
async fn adds_the_retrieve_tool_tool_injection() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, "");
    let output = apply_as_caller(&guardrail, conversation(TOOL_CONVERSATION, "")).await;
    let tools = output.tools.unwrap();
    assert_eq!(names(&tools), ["compresr_retrieve"]);
    let added = tools.first().unwrap();
    assert_eq!(added.tool_type, "function");
    let parameters = field(&function_of(added), "parameters");
    assert_eq!(
        field(&parameters, "required"),
        GuardrailValue::Array(vec![GuardrailValue::Text("hash".to_owned())])
    );
}

#[rstest]
#[case::the_callers_tools_stay_first(SEARCH, &["search", "compresr_retrieve"])]
#[case::an_existing_retrieve_tool_is_not_repeated(RETRIEVE, &["compresr_retrieve"])]
#[tokio::test]
async fn merges_with_the_callers_tools_tool_injection(
    #[case] existing: &str,
    #[case] expected: &[&str],
) {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, "");
    let extra = format!(r#","tools":[{existing}]"#);
    let output = apply_as_caller(&guardrail, conversation(TOOL_CONVERSATION, &extra)).await;
    assert_eq!(names(&output.tools.unwrap()), expected);
}

#[rstest]
#[case::retrieval_off("      enable_retrieval: false\n")]
#[case::nothing_stored("      max_bytes_per_call: 1\n")]
#[tokio::test]
async fn left_alone_without_a_stored_original_tool_injection(#[case] params: &str) {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, params);
    let extra = format!(r#","tools":[{SEARCH}]"#);
    let output = apply_as_caller(&guardrail, conversation(TOOL_CONVERSATION, &extra)).await;
    assert_eq!(names(&output.tools.unwrap()), ["search"]);
}

#[rstest]
#[case::no_tools(&[], &["compresr_retrieve"])]
#[case::appended_last(&[SEARCH], &["search", "compresr_retrieve"])]
#[case::already_there(&[RETRIEVE, SEARCH], &["compresr_retrieve", "search"])]
fn appends_once_merge_tool(#[case] existing: &[&str], #[case] expected: &[&str]) {
    let tools: Vec<GuardrailToolParam> = existing
        .iter()
        .map(|json| serde_json::from_str(json).unwrap())
        .collect();
    let merged = merged_tools(Some(tools.as_slice())).unwrap();
    assert_eq!(names(&merged), expected);
    assert_eq!(names(&merged_tools(None).unwrap()), ["compresr_retrieve"]);
}
