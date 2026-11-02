#![cfg(test)]

use super::RowsError;
use crate::sources::count_row::CountRow;

#[test]
fn decodes_a_list_of_rows() {
    let rows: Vec<CountRow> = serde_json::from_slice(br#"[{"count": 1}, {"count": 2}]"#)
        .map_err(RowsError)
        .expect("decodes");
    assert_eq!(rows, vec![CountRow { count: 1 }, CountRow { count: 2 }]);
}

#[test]
fn decodes_an_empty_list() {
    let rows: Vec<CountRow> = serde_json::from_slice(b"[]")
        .map_err(RowsError)
        .expect("decodes");
    assert!(rows.is_empty());
}

#[test]
fn rejects_malformed_json() {
    let error: Result<Vec<CountRow>, RowsError> =
        serde_json::from_slice(b"not json").map_err(RowsError);
    error.unwrap_err();
}

#[test]
fn rejects_a_row_with_the_wrong_shape() {
    let error: Result<Vec<CountRow>, RowsError> =
        serde_json::from_slice(br#"[{"count": "one"}]"#).map_err(RowsError);
    error.unwrap_err();
}
