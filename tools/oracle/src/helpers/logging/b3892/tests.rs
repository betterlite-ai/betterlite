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
