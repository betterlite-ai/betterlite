#![cfg(test)]

#[test]
fn helper_functions_dispatch() {
    use crate::helpers::Helper;
    let input = serde_json::json!({"function":"cache_token_value","values":[0,4]});
    assert_eq!(super::Op::run(&input).unwrap(), serde_json::json!(4));
}
