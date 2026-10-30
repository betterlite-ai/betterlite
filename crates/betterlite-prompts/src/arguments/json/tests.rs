#![cfg(test)]

use super::Json;
use serde_json::Number;

#[test]
fn a_nonzero_number_is_truthy() {
    assert!(Json::Number(Number::from(1_i32)).is_truthy());
}

#[test]
fn a_zero_number_is_not_truthy() {
    assert!(!Json::Number(Number::from(0_i32)).is_truthy());
}

#[test]
fn a_nonempty_text_is_truthy() {
    assert!(Json::Text("value".to_owned()).is_truthy());
}

#[test]
fn an_empty_text_is_not_truthy() {
    assert!(!Json::Text(String::new()).is_truthy());
}

#[test]
fn a_nonempty_list_is_truthy() {
    assert!(Json::List(vec![Json::Null]).is_truthy());
}

#[test]
fn an_empty_list_is_not_truthy() {
    assert!(!Json::List(Vec::new()).is_truthy());
}
