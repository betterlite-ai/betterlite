use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::team_rows::member::BudgetLimitEntry;

use super::{decode_value, json_data, object};
use crate::key_fields::error::DecodeError;
use crate::key_fields::windows::{stored_budget_limit_windows, typed_budget_limit_windows};

#[test]
fn stored_windows_empty() {
    assert_eq!(
        stored_budget_limit_windows(None).expect("no stored value"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("null"))).expect("null"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("false"))).expect("false"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("0"))).expect("zero"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data(r#""""#))).expect("empty text"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("[]"))).expect("empty list"),
        Vec::new()
    );
    assert_eq!(
        stored_budget_limit_windows(Some(&json_data("{}"))).expect("empty object"),
        Vec::new()
    );
}

#[test]
fn stored_windows_json() {
    let stored = JsonData::String(r#"[{"budget_duration": "1d", "max_budget": 2.5}]"#.to_owned());
    let windows = stored_budget_limit_windows(Some(&stored)).expect("JSON text");
    assert_eq!(
        windows,
        vec![object(r#"{"budget_duration": "1d", "max_budget": 2.5}"#)]
    );
    let listed = json_data(r#"[{"budget_duration": "1d", "max_budget": 2.5}]"#);
    let same = stored_budget_limit_windows(Some(&listed)).expect("a parsed list");
    assert_eq!(same, windows);
}

#[test]
fn stored_windows_typed() {
    let entry: BudgetLimitEntry = decode_value(&json_data(
        r#"{"budget_duration": "1d", "max_budget": 2.5}"#,
    ))
    .expect("a typed window");
    let windows = typed_budget_limit_windows(Some(&[entry])).expect("typed entries");
    assert_eq!(
        windows,
        vec![object(
            r#"{"budget_duration": "1d", "max_budget": 2.5, "reset_at": null}"#
        )]
    );
    assert_eq!(
        typed_budget_limit_windows(Some(&[])).expect("no entries"),
        Vec::new()
    );
    assert_eq!(
        typed_budget_limit_windows(None).expect("no list"),
        Vec::new()
    );
}

#[test]
fn stored_windows_invalid() {
    let broken = JsonData::String("{bad".to_owned());
    assert!(matches!(
        stored_budget_limit_windows(Some(&broken)),
        Err(DecodeError::StoredWindowsJson(_))
    ));
    let not_a_list = JsonData::String("null".to_owned());
    assert!(matches!(
        stored_budget_limit_windows(Some(&not_a_list)),
        Err(DecodeError::StoredWindowsNotAList)
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data("true"))),
        Err(DecodeError::StoredWindowsNotAList)
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data("5"))),
        Err(DecodeError::StoredWindowsNotAList)
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data(r#"{"a": 1}"#))),
        Err(DecodeError::StoredWindowsNotAList)
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data(r#"["x"]"#))),
        Err(DecodeError::StoredWindowsUnsupported { index: 0 })
    ));
    assert!(matches!(
        stored_budget_limit_windows(Some(&json_data(r#"[{}, "x"]"#))),
        Err(DecodeError::StoredWindowsUnsupported { index: 1 })
    ));
}
