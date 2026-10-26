#![cfg(test)]

use super::{TransformError, UsageRow, transform};
use crate::focus_records::schema::{ColumnType, FOCUS_NORMALIZED_SCHEMA};
use crate::focus_records::table::{ColumnValues, FocusTable};
use rstest::rstest;
use rust_decimal::Decimal;

fn usage() -> UsageRow {
    UsageRow {
        date: "2024-03-05".to_owned(),
        api_key: "hashed-key".to_owned(),
        spend: 1.5,
        api_requests: 3,
        user_id: Some("user-1".to_owned()),
        model: Some("gpt-4o".to_owned()),
        model_group: Some("gpt-4o-group".to_owned()),
        custom_llm_provider: Some("openai".to_owned()),
        team_id: Some("team-1".to_owned()),
        team_alias: Some("Search".to_owned()),
        organization_id: None,
        organization_alias: None,
        user_email: Some("ana@example.com".to_owned()),
        api_key_alias: Some("prod-key".to_owned()),
    }
}

fn column<'table>(table: &'table FocusTable, name: &str) -> &'table ColumnValues {
    &table
        .columns
        .iter()
        .find(|column| column.name == name)
        .unwrap()
        .values
}

fn text(table: &FocusTable, name: &str) -> Vec<Option<String>> {
    let ColumnValues::Text(values) = column(table, name) else {
        panic!("{name} is not text");
    };
    values.clone()
}

fn decimal(table: &FocusTable, name: &str) -> Vec<Option<Decimal>> {
    let ColumnValues::Decimal(values) = column(table, name) else {
        panic!("{name} is not decimal");
    };
    values.clone()
}

const NONEMPTY_COLUMNS: [(&str, ColumnType); 44] = [
    ("BilledCost", ColumnType::Decimal),
    ("BillingAccountId", ColumnType::Text),
    ("BillingAccountName", ColumnType::Text),
    ("BillingAccountType", ColumnType::Text),
    ("BillingCurrency", ColumnType::Text),
    ("BillingPeriodEnd", ColumnType::Text),
    ("BillingPeriodStart", ColumnType::Text),
    ("ChargeCategory", ColumnType::Text),
    ("ChargeClass", ColumnType::Text),
    ("ChargeDescription", ColumnType::Text),
    ("ChargeFrequency", ColumnType::Text),
    ("ChargePeriodEnd", ColumnType::Text),
    ("ChargePeriodStart", ColumnType::Text),
    ("ConsumedQuantity", ColumnType::Decimal),
    ("ConsumedUnit", ColumnType::Text),
    ("ContractedCost", ColumnType::Decimal),
    ("ContractedUnitPrice", ColumnType::Text),
    ("EffectiveCost", ColumnType::Decimal),
    ("InvoiceIssuerName", ColumnType::Text),
    ("InvoiceId", ColumnType::Text),
    ("ListCost", ColumnType::Decimal),
    ("ListUnitPrice", ColumnType::Decimal),
    ("AvailabilityZone", ColumnType::Text),
    ("PricingCurrency", ColumnType::Text),
    ("PricingCategory", ColumnType::Text),
    ("PricingQuantity", ColumnType::Decimal),
    ("PricingCurrencyContractedUnitPrice", ColumnType::Decimal),
    ("PricingCurrencyEffectiveCost", ColumnType::Decimal),
    ("PricingCurrencyListUnitPrice", ColumnType::Decimal),
    ("PricingUnit", ColumnType::Text),
    ("ProviderName", ColumnType::Text),
    ("PublisherName", ColumnType::Text),
    ("RegionId", ColumnType::Text),
    ("RegionName", ColumnType::Text),
    ("ResourceId", ColumnType::Text),
    ("ResourceName", ColumnType::Text),
    ("ResourceType", ColumnType::Text),
    ("ServiceCategory", ColumnType::Text),
    ("ServiceSubcategory", ColumnType::Text),
    ("ServiceName", ColumnType::Text),
    ("SubAccountId", ColumnType::Text),
    ("SubAccountName", ColumnType::Text),
    ("SubAccountType", ColumnType::Text),
    ("Tags", ColumnType::Text),
];

#[test]
fn nonempty_schema_has_44_columns_with_the_seven_computed_ones() {
    let table = transform(&[usage(), usage()]).unwrap();
    let columns: Vec<(&str, ColumnType)> = table
        .columns
        .iter()
        .map(|column| (column.name, column.values.kind()))
        .collect();
    assert_eq!(columns, NONEMPTY_COLUMNS.to_vec());
    assert_eq!(table.rows, 2);
}

#[test]
fn empty_rows_adopt_the_37_column_schema() {
    let table = transform(&[]).unwrap();
    let columns: Vec<(&str, ColumnType)> = table
        .columns
        .iter()
        .map(|column| (column.name, column.values.kind()))
        .collect();
    assert_eq!(columns, FOCUS_NORMALIZED_SCHEMA.to_vec());
    assert_eq!(table.rows, 0);
}

#[test]
fn tags_follow_litellm_key_order_and_skip_nulls() {
    let table = transform(&[usage()]).unwrap();
    let expected = concat!(
        r#"{"team_id": "team-1", "team_alias": "Search", "user_id": "user-1", "#,
        r#""user_email": "ana@example.com", "api_key_alias": "prod-key", "model": "gpt-4o", "#,
        r#""model_group": "gpt-4o-group", "custom_llm_provider": "openai"}"#
    );
    assert_eq!(text(&table, "Tags"), vec![Some(expected.to_owned())]);
}

#[rstest]
#[case::no_metadata(None, "{}")]
#[case::quote_and_newline(Some("a\"b\nc"), r#"{"team_alias": "a\"b\nc"}"#)]
#[case::controls(
    Some("\\\t\r\u{8}\u{c}\u{1}\u{7f}"),
    r#"{"team_alias": "\\\t\r\b\f\u0001\u007f"}"#
)]
#[case::non_ascii(
    Some("\u{c9}quipe \u{1f680}"),
    "{\"team_alias\": \"\\u00c9quipe \\ud83d\\ude80\"}"
)]
fn tags_match_python_json_dumps(#[case] alias: Option<&str>, #[case] expected: &str) {
    let row = UsageRow {
        user_id: None,
        model: None,
        model_group: None,
        custom_llm_provider: None,
        team_id: None,
        team_alias: alias.map(str::to_owned),
        user_email: None,
        api_key_alias: None,
        ..usage()
    };
    let table = transform(&[row]).unwrap();
    assert_eq!(text(&table, "Tags"), vec![Some(expected.to_owned())]);
}

#[rstest]
#[case::charge_start("ChargePeriodStart", "2024-03-05T00:00:00Z")]
#[case::charge_end("ChargePeriodEnd", "2024-03-06T00:00:00Z")]
#[case::billing_start("BillingPeriodStart", "2024-03-05T00:00:00Z")]
#[case::billing_end("BillingPeriodEnd", "2024-03-06T00:00:00Z")]
fn periods_are_utc_strings_one_day_long(#[case] name: &str, #[case] expected: &str) {
    let table = transform(&[usage()]).unwrap();
    assert_eq!(text(&table, name), vec![Some(expected.to_owned())]);
}

#[test]
fn unparsable_date_leaves_periods_null() {
    let row = UsageRow {
        date: "05/03/2024".to_owned(),
        ..usage()
    };
    let table = transform(&[row]).unwrap();
    assert_eq!(text(&table, "ChargePeriodStart"), vec![None]);
    assert_eq!(text(&table, "BillingPeriodEnd"), vec![None]);
}

#[rstest]
#[case::billed("BilledCost", Some(Decimal::new(1_500_000, 6)))]
#[case::contracted("ContractedCost", Some(Decimal::new(1_500_000, 6)))]
#[case::effective("EffectiveCost", Some(Decimal::new(1_500_000, 6)))]
#[case::list("ListCost", Some(Decimal::new(1_500_000, 6)))]
#[case::pricing_effective("PricingCurrencyEffectiveCost", Some(Decimal::new(1_500_000, 6)))]
#[case::consumed("ConsumedQuantity", Some(Decimal::new(3, 0)))]
#[case::pricing_quantity("PricingQuantity", Some(Decimal::new(3, 0)))]
#[case::list_unit_price("ListUnitPrice", None)]
#[case::pricing_list_unit_price("PricingCurrencyListUnitPrice", None)]
fn money_and_quantities_are_decimals(#[case] name: &str, #[case] expected: Option<Decimal>) {
    let table = transform(&[usage()]).unwrap();
    assert_eq!(decimal(&table, name), vec![expected]);
}

#[rstest]
#[case::account_id("BillingAccountId", Some("hashed-key"))]
#[case::account_name("BillingAccountName", Some("prod-key"))]
#[case::account_type("BillingAccountType", Some("API Key"))]
#[case::currency("BillingCurrency", Some("USD"))]
#[case::category("ChargeCategory", Some("Usage"))]
#[case::class("ChargeClass", None)]
#[case::description("ChargeDescription", Some("gpt-4o"))]
#[case::frequency("ChargeFrequency", Some("Usage-Based"))]
#[case::consumed_unit("ConsumedUnit", Some("Requests"))]
#[case::contracted_unit_price("ContractedUnitPrice", None)]
#[case::issuer("InvoiceIssuerName", Some("openai"))]
#[case::invoice("InvoiceId", None)]
#[case::zone("AvailabilityZone", None)]
#[case::pricing_currency("PricingCurrency", Some("USD"))]
#[case::pricing_unit("PricingUnit", Some("Requests"))]
#[case::provider("ProviderName", Some("openai"))]
#[case::publisher("PublisherName", Some("openai"))]
#[case::region("RegionName", None)]
#[case::resource_type("ResourceType", Some("gpt-4o"))]
#[case::service_category("ServiceCategory", Some("AI and Machine Learning"))]
#[case::service_subcategory("ServiceSubcategory", Some("Generative AI"))]
#[case::service("ServiceName", Some("gpt-4o-group"))]
#[case::sub_account("SubAccountId", Some("team-1"))]
#[case::sub_account_name("SubAccountName", Some("Search"))]
#[case::sub_account_type("SubAccountType", None)]
fn text_columns_project_usage_fields(#[case] name: &str, #[case] expected: Option<&str>) {
    let table = transform(&[usage()]).unwrap();
    assert_eq!(text(&table, name), vec![expected.map(str::to_owned)]);
}

#[rstest]
#[case::ties_round_to_even(0.000_037_5_f64, Decimal::new(38, 6))]
#[case::ties_round_down_to_even(0.000_012_5_f64, Decimal::new(12, 6))]
#[case::below_half(0.000_000_4_f64, Decimal::ZERO)]
#[case::largest_fit(999_999_999_999.5_f64, Decimal::new(999_999_999_999_500_000, 6))]
fn spend_rounds_to_six_places(#[case] spend: f64, #[case] expected: Decimal) {
    let row = UsageRow { spend, ..usage() };
    let table = transform(&[row]).unwrap();
    assert_eq!(decimal(&table, "BilledCost"), vec![Some(expected)]);
}

#[rstest]
#[case::spend_overflow(1e12_f64, 0_i64)]
#[case::requests_overflow(0.0_f64, 1_000_000_000_000_i64)]
fn values_past_decimal_18_6_fail(#[case] spend: f64, #[case] api_requests: i64) {
    let row = UsageRow {
        spend,
        api_requests,
        ..usage()
    };
    let error = transform(&[row]).unwrap_err();
    assert!(
        matches!(error, TransformError::Overflow { .. }),
        "{error:?}"
    );
}

#[test]
fn spend_that_is_not_a_number_fails() {
    let row = UsageRow {
        spend: f64::NAN,
        ..usage()
    };
    let error = transform(&[row]).unwrap_err();
    assert!(matches!(error, TransformError::Spend { .. }), "{error:?}");
}
