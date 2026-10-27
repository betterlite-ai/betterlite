#![cfg(test)]

use super::{AssemblyInput, assemble};
use rstest::rstest;

#[rstest]
#[case::tool_indices_and_fragments(r#"{"tool":{"chunks":[{"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"a","function":{"name":"first","arguments":"a"}},{"index":0,"id":"b","function":{"name":"second","arguments":"{}"}}]}}]},{"choices":[{"index":1,"delta":{"tool_calls":[{"index":1,"id":"c","function":{"name":"third","arguments":"x"}}]}}]},{"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"b"}}]}}]}]}}"#, r#"[{"id":"b","type":"function","function":{"name":"second","arguments":"{}"}},{"id":"a","type":"function","function":{"name":"first","arguments":"ab"}},{"id":"c","type":"function","function":{"name":"third","arguments":"x"}}]"#)]
#[case::tool_metadata_last_truthy(r#"{"tool":{"chunks":[{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","type":"function","function":{"name":"one","provider_specific_fields":{"a":1}}}]}}]},{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a2","type":"custom","function":{"name":"two","provider_specific_fields":{"b":2}},"provider_specific_fields":{"a":3}}]}}]}]}}"#, r#"[{"id":"a2","type":"custom","function":{"name":"two","arguments":"{}"},"provider_specific_fields":{"a":3}}]"#)]
#[case::custom_precedence_and_incomplete(r#"{"tool":{"chunks":[{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"custom-id","function":{"name":"ignored","arguments":"x"},"custom":{"name":"custom","input":"a"}}]}}]},{"choices":[{"delta":{"tool_calls":[{"index":0,"custom":{"input":"b"}},{"index":1,"function":{"name":"missing-id"}}]}}]}]}}"#, r#"[{"id":"custom-id","type":"custom","custom":{"name":"custom","input":"ab"}}]"#)]
#[case::legacy_function_join(r#"{"function":{"chunks":[{"choices":[{"delta":{"function_call":{"name":"first","arguments":"a"}}}]},{"choices":[{"delta":{"function_call":{"name":"second","arguments":"b"}}}]}]}}"#, r#"{"name":"first","arguments":"ab"}"#)]
fn stream_tool_assembly_cases(#[case] input: &str, #[case] expected: &str) {
    let decoded: AssemblyInput = serde_json::from_str(input).unwrap();
    let expected_output = expected.to_owned();
    let actual = serde_json::to_string(&assemble(&decoded).unwrap()).unwrap();
    assert_eq!(actual, expected_output);
}
