#![cfg(test)]

use std::collections::BTreeMap;

use crate::mcp_stored_contracts::JsonValue;

use super::safe_dumps;

fn nested(levels: u8) -> JsonValue {
    (0_u8..levels).fold(JsonValue::Null, |inner, _| JsonValue::Array(vec![inner]))
}

#[test]
fn create_dump_strips_nul_bytes_from_keys_and_text() {
    let mut data = BTreeMap::new();
    data.insert("a\u{0}b".to_owned(), "c\u{0}d".to_owned());
    assert_eq!(safe_dumps(&data).unwrap(), r#"{"ab":"cd"}"#);
}

#[test]
fn create_dump_walks_arrays_objects_and_scalars() {
    let data: JsonValue =
        serde_json::from_str(r#"{"a":[1,true,null,"x\u0000y",{"k\u0000":"v"}]}"#).unwrap();
    assert_eq!(
        safe_dumps(&data).unwrap(),
        r#"{"a":[1,true,null,"xy",{"k":"v"}]}"#
    );
}

#[test]
fn create_dump_accepts_the_deepest_stored_document_and_refuses_one_more() {
    assert!(safe_dumps(&nested(64)).is_ok());
    let error = safe_dumps(&nested(65)).unwrap_err();
    assert!(error.to_string().contains("exceeds depth 64"));
}
