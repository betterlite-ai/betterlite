use std::collections::BTreeMap;

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_repository::config::ConfigParam;

use super::{assert_field, decode_value, json_data, object};
use crate::key_fields::adapters::{KeyListUserRow, LegacyDumpable, env_vars_param_value};
use crate::key_fields::decode::to_json;
use crate::key_fields::values::{InsensitiveContains, MatchMode};

#[test]
fn user_row_view() {
    let row: LiteLLMUserTable = serde_json::from_str(
        r#"{"user_id": "u", "user_alias": "al", "user_email": "e@example.com", "spend": 1.5}"#,
    )
    .expect("a user row");
    assert_eq!(row.user_id(), "u");
    assert_eq!(row.user_alias(), Some("al"));
    assert_eq!(row.user_email(), Some("e@example.com"));
    let dumped = row.dumped().expect("the row dump");
    assert_field(&dumped, "user_id", r#""u""#);
    assert_field(&dumped, "user_alias", r#""al""#);
}

#[derive(serde::Serialize)]
struct LegacyRow {
    id: String,
}

#[test]
fn legacy_dump() {
    let row = LegacyRow { id: "r".to_owned() };
    assert_eq!(
        row.legacy_dict().expect("the legacy dump"),
        object(r#"{"id": "r"}"#)
    );
}

#[test]
fn env_vars_param() {
    let param = ConfigParam {
        param_name: "environment_variables".to_owned(),
        param_value: json_data(r#"{"A": "1", "B": "2"}"#),
    };
    assert_eq!(
        env_vars_param_value(&param),
        Some(BTreeMap::from([
            ("A".to_owned(), "1".to_owned()),
            ("B".to_owned(), "2".to_owned()),
        ]))
    );
    let not_text = ConfigParam {
        param_name: "environment_variables".to_owned(),
        param_value: json_data(r#"{"A": 1}"#),
    };
    assert_eq!(env_vars_param_value(&not_text), None);
}

#[test]
fn insensitive_contains() {
    let filter = decode_value::<InsensitiveContains>(&json_data(
        r#"{"contains": "abc", "mode": "insensitive"}"#,
    ))
    .expect("the search filter");
    assert_eq!(filter.contains, "abc");
    assert_eq!(filter.mode, MatchMode::Insensitive);
    assert!(decode_value::<InsensitiveContains>(&json_data(r#"{"contains": "abc"}"#)).is_err());
    assert!(
        decode_value::<InsensitiveContains>(&json_data(
            r#"{"contains": "abc", "mode": "sensitive"}"#
        ))
        .is_err()
    );
    assert_eq!(
        to_json(&filter).expect("the filter dump"),
        json_data(r#"{"contains": "abc", "mode": "insensitive"}"#)
    );
}
