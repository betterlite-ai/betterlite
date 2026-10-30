#![cfg(test)]

use super::{Input, Op};
use crate::helpers::Helper as _;
use rstest::rstest;
use serde_json::Value;

#[rstest]
#[case::estimate_json_bytes_counts_a_short_string(
    r#"{"mode":"estimate_json_bytes","value":"ab"}"#,
    serde_json::json!(4_i32)
)]
#[case::unpack_defs_expands_a_local_reference(
    r##"{"mode":"unpack_defs","schema":{"a":{"$ref":"#/$defs/Foo"}},"defs":{"Foo":{"type":"string"}}}"##,
    serde_json::json!({"a": {"type": "string"}})
)]
#[case::has_legacy_defs_detects_a_definitions_key(
    r#"{"mode":"has_legacy_defs","schema":{"definitions":{}}}"#,
    serde_json::json!(true)
)]
#[case::unpack_legacy_defs_inlines_a_definitions_backed_ref(
    r##"{"mode":"unpack_legacy_defs","schema":{"definitions":{"Foo":{"type":"string"}},"a":{"$ref":"#/$defs/Foo"}}}"##,
    serde_json::json!({"a": {"type": "string"}})
)]
#[case::sanitize_input_schema_for_anthropic_forces_object_type(
    r#"{"mode":"sanitize_input_schema_for_anthropic","input_schema":{"properties":{}}}"#,
    serde_json::json!({"type": "object", "properties": {}})
)]
fn schema_definitions_oracle_adapter_tests(#[case] request: &str, #[case] expected: Value) {
    let input: Input = serde_json::from_str(request).expect("request is valid JSON");
    let value = Op::run(&input).expect("case is expected to succeed");
    assert_eq!(value, expected);
}
