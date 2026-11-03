#![cfg(test)]
use super::Op;
use crate::helpers::Helper as _;
use serde_json::json;

#[test]
fn trace_helper_serializes_controls() {
    let input = json!({"function":"trace","litellm_params":{"metadata":{"trace_name":"name"}}});
    let output = Op::run(&input).expect("trace helper succeeds");
    assert_eq!(
        output,
        json!({"name":"name","user_id":null,"session_id":null,"tags":[]})
    );
}

#[test]
fn coercion_helper_preserves_decimal_text_and_tuple_shape() {
    let input = json!({"function":"coercion","values":[
        {"kind":"decimal","value":"1.250"},
        {"kind":"tuple","value":["one","two"]}
    ]});
    let output = Op::run(&input).expect("coercion helper succeeds");
    assert_eq!(output, json!(["1.25", ["one", "two"]]));
}

#[test]
fn coercion_helper_uses_production_boolean_and_mapping_accessors() {
    let input = json!({"function":"coercion","values":[
        {"kind":"boolean","value":true},
        {"kind":"boolean","value":false},
        {"kind":"boolean","value":"false"},
        {"kind":"mapping","value":{"key":3_i64}},
        {"kind":"mapping","value":[3_i64]}
    ]});
    let output = Op::run(&input).expect("coercion helper succeeds");
    assert_eq!(output, json!([true, false, null, {"key": 3_i64}, null]));
}
