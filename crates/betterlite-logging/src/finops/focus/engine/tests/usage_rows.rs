use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::finops::focus::UsageData;
use rust_decimal::Decimal;

use crate::finops::focus::engine::rows::{UsageRowError, decode};

use super::support::{data_row, number, text};

fn row(
    date: JsonData,
    api_key: JsonData,
    spend: JsonData,
    api_requests: JsonData,
) -> BTreeMap<String, JsonData> {
    data_row(&[
        ("date", date),
        ("api_key", api_key),
        ("spend", spend),
        ("api_requests", api_requests),
    ])
}

fn complete_row() -> BTreeMap<String, JsonData> {
    row(
        text("2026-03-04"),
        text("api-key-hash"),
        number("12.345678"),
        number("7"),
    )
}

#[test]
fn decode_reads_the_required_columns() {
    let rows: UsageData = vec![complete_row()];
    let decoded = decode(&rows).expect("the row decodes");
    let row = decoded.first().expect("one row decodes");
    assert_eq!(row.date, "2026-03-04");
    assert_eq!(row.api_key, "api-key-hash");
    assert_eq!(
        row.spend,
        Decimal::from_str("12.345678").expect("the spend is a decimal")
    );
    assert_eq!(row.api_requests, 7);
    assert_eq!(row.user_id, None);
    assert_eq!(row.model, None);
    assert_eq!(row.api_key_alias, None);
}

#[test]
fn decode_reads_the_optional_columns() {
    let mut record = complete_row();
    record.insert(String::from("user_id"), text("user-1"));
    record.insert(String::from("model"), text("gpt-4o"));
    record.insert(String::from("team_id"), text("team-1"));
    record.insert(String::from("user_email"), text("user@example.com"));
    record.insert(String::from("api_key_alias"), text("key-alias"));
    let rows: UsageData = vec![record];
    let decoded = decode(&rows).expect("the row decodes");
    let row = decoded.first().expect("one row decodes");
    assert_eq!(row.user_id.as_deref(), Some("user-1"));
    assert_eq!(row.model.as_deref(), Some("gpt-4o"));
    assert_eq!(row.team_id.as_deref(), Some("team-1"));
    assert_eq!(row.user_email.as_deref(), Some("user@example.com"));
    assert_eq!(row.api_key_alias.as_deref(), Some("key-alias"));
    assert_eq!(row.team_alias, None);
}

#[test]
fn decode_treats_a_null_optional_value_as_absent() {
    let mut record = complete_row();
    record.insert(String::from("model"), JsonData::Null);
    let rows: UsageData = vec![record];
    let decoded = decode(&rows).expect("the row decodes");
    let row = decoded.first().expect("one row decodes");
    assert_eq!(row.model, None);
}

#[test]
fn decode_keeps_the_exact_spend_scale() {
    let rows: UsageData = vec![row(
        text("2026-03-04"),
        text("api-key-hash"),
        number("1234.567890"),
        number("1"),
    )];
    let decoded = decode(&rows).expect("the row decodes");
    let row = decoded.first().expect("one row decodes");
    assert_eq!(row.spend.to_string(), "1234.567890");
}

#[test]
fn decode_reports_a_missing_required_value() {
    let rows: UsageData = vec![row(
        JsonData::Null,
        text("api-key-hash"),
        number("1"),
        number("1"),
    )];
    let error = decode(&rows).err().expect("the date is missing");
    assert!(matches!(
        error,
        UsageRowError::Missing {
            index: 0,
            field: "date"
        }
    ));
}

#[test]
fn decode_reports_a_wrong_type() {
    let rows: UsageData = vec![row(
        number("2026"),
        text("api-key-hash"),
        number("1"),
        number("1"),
    )];
    let error = decode(&rows).err().expect("the date is not text");
    assert!(matches!(
        error,
        UsageRowError::WrongType {
            index: 0,
            field: "date"
        }
    ));
}

#[test]
fn decode_reports_requests_that_are_not_an_integer() {
    let rows: UsageData = vec![row(
        text("2026-03-04"),
        text("api-key-hash"),
        number("1"),
        text("7"),
    )];
    let error = decode(&rows)
        .err()
        .expect("the request count is not a number");
    assert!(matches!(
        error,
        UsageRowError::WrongType {
            index: 0,
            field: "api_requests"
        }
    ));
}

#[test]
fn decode_reports_a_spend_that_is_not_a_decimal() {
    let huge = "9999999999999999999999999999999999999999";
    let rows: UsageData = vec![row(
        text("2026-03-04"),
        text("api-key-hash"),
        number(huge),
        number("1"),
    )];
    let error = decode(&rows)
        .err()
        .expect("the spend does not fit a decimal");
    assert!(matches!(
        error,
        UsageRowError::NotDecimal {
            index: 0,
            field: "spend"
        }
    ));
}

#[test]
fn decode_reports_the_row_index() {
    let rows: UsageData = vec![
        complete_row(),
        row(
            JsonData::Null,
            text("api-key-hash"),
            number("1"),
            number("1"),
        ),
    ];
    let error = decode(&rows).err().expect("the second row has no date");
    assert!(matches!(
        error,
        UsageRowError::Missing {
            index: 1,
            field: "date"
        }
    ));
}

#[test]
fn decode_returns_no_rows_for_an_empty_usage_data() {
    let rows: UsageData = Vec::new();
    assert!(decode(&rows).expect("an empty query decodes").is_empty());
}
