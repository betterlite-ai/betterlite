#![cfg(test)]

#[test]
fn helper_functions_dispatch() {
    let input = serde_json::json!({"function":"cache_token_value","values":[0_i64,4_i64]});
    let result = <super::Op as crate::helpers::Helper>::run(&input).unwrap();
    assert_eq!(result, serde_json::json!(4_i64));
}
