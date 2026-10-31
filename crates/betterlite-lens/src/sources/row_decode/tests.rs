#![cfg(test)]

use super::decode_rows;
use crate::sources::count_row::CountRow;

#[test]
fn decodes_a_list_of_rows() {
    let rows: Vec<CountRow> = decode_rows(br#"[{"count": 1}, {"count": 2}]"#).expect("decodes");
    assert_eq!(rows, vec![CountRow { count: 1 }, CountRow { count: 2 }]);
}

#[test]
fn decodes_an_empty_list() {
    let rows: Vec<CountRow> = decode_rows(b"[]").expect("decodes");
    assert!(rows.is_empty());
}

#[test]
fn rejects_malformed_json() {
    let error = decode_rows::<CountRow>(b"not json");
    error.unwrap_err();
}

#[test]
fn rejects_a_row_with_the_wrong_shape() {
    let error = decode_rows::<CountRow>(br#"[{"count": "one"}]"#);
    error.unwrap_err();
}
