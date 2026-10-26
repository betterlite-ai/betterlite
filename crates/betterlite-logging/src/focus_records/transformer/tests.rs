#![cfg(test)]

use super::{TransformError, UsageRow, transform};
use crate::focus_records::schema::{ColumnType, FOCUS_NORMALIZED_SCHEMA};
use crate::focus_records::serializers::FocusFormat;
use crate::focus_records::table::ColumnValues;
use rstest::rstest;
use rust_decimal::Decimal;

fn usage() -> UsageRow {
    UsageRow {
        date: "2024-03-05".to_owned(),
        api_key: "key".to_owned(),
        spend: Decimal::new(25, 2),
        api_requests: 2,
        user_id: Some("user-1".to_owned()),
        model: Some("m".to_owned()),
        model_group: Some("group".to_owned()),
        custom_llm_provider: Some("openai".to_owned()),
        team_id: Some("t".to_owned()),
        team_alias: Some("Search".to_owned()),
        organization_id: None,
        organization_alias: None,
        user_email: Some("ana@example.com".to_owned()),
        api_key_alias: Some("prod".to_owned()),
    }
}

fn column(row: UsageRow, name: &str) -> ColumnValues {
    let table = transform(&[row]).unwrap();
    let found = table.columns.into_iter().find(|column| column.name == name);
    found.unwrap().values
}

#[test]
fn nonempty_schema_has_44_columns_with_the_seven_computed_ones() {
    let table = transform(&[usage(), usage()]).unwrap();
    let render = |kind: ColumnType| -> String {
        let names = table
            .columns
            .iter()
            .filter(|column| column.values.kind() == kind);
        names
            .map(|column| column.name)
            .collect::<Vec<_>>()
            .join(",")
    };
    let decimals = concat!(
        "BilledCost,ConsumedQuantity,ContractedCost,EffectiveCost,ListCost,ListUnitPrice,PricingQuantity,",
        "PricingCurrencyContractedUnitPrice,PricingCurrencyEffectiveCost,PricingCurrencyListUnitPrice"
    );
    let texts = concat!(
        "BillingAccountId,BillingAccountName,BillingAccountType,BillingCurrency,BillingPeriodEnd,",
        "BillingPeriodStart,ChargeCategory,ChargeClass,ChargeDescription,ChargeFrequency,ChargePeriodEnd,",
        "ChargePeriodStart,ConsumedUnit,ContractedUnitPrice,InvoiceIssuerName,InvoiceId,AvailabilityZone,",
        "PricingCurrency,PricingCategory,PricingUnit,ProviderName,PublisherName,RegionId,RegionName,",
        "ResourceId,ResourceName,ResourceType,ServiceCategory,ServiceSubcategory,ServiceName,SubAccountId,",
        "SubAccountName,SubAccountType,Tags"
    );
    assert_eq!(
        (render(ColumnType::Decimal), render(ColumnType::Text)),
        (decimals.to_owned(), texts.to_owned())
    );
    assert_eq!((table.columns.len(), table.rows), (44, 2));
    assert_eq!(
        table.columns.get(3).map(|column| column.name),
        Some("BillingAccountType")
    );
}

#[test]
fn empty_rows_adopt_the_37_column_schema() {
    let table = transform(&[]).unwrap();
    let columns: Vec<(&str, ColumnType)> = table
        .columns
        .iter()
        .map(|column| (column.name, column.values.kind()))
        .collect();
    assert_eq!((columns, table.rows), (FOCUS_NORMALIZED_SCHEMA.to_vec(), 0));
}

#[test]
fn every_column_projects_its_usage_field() {
    let bytes = FocusFormat::Csv
        .encode(&transform(&[usage()]).unwrap())
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let expected = concat!(
        "0.25,key,prod,API Key,USD,2024-03-06T00:00:00Z,2024-03-05T00:00:00Z,Usage,,m,Usage-Based,",
        "2024-03-06T00:00:00Z,2024-03-05T00:00:00Z,2.0,Requests,0.25,,0.25,openai,,0.25,,,USD,,2.0,,0.25,,",
        "Requests,openai,openai,,,m,m,m,AI and Machine Learning,Generative AI,group,t,Search,,",
        "\"{\"\"team_id\"\": \"\"t\"\", \"\"team_alias\"\": \"\"Search\"\", \"\"user_id\"\": \"\"user-1\"\", ",
        "\"\"user_email\"\": \"\"ana@example.com\"\", \"\"api_key_alias\"\": \"\"prod\"\", \"\"model\"\": \"\"m\"\", ",
        "\"\"model_group\"\": \"\"group\"\", \"\"custom_llm_provider\"\": \"\"openai\"\"}\"",
    );
    assert_eq!(text.lines().nth(1), Some(expected));
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
    assert_eq!(
        column(row, "Tags"),
        ColumnValues::Text(vec![Some(expected.to_owned())])
    );
}

#[test]
fn unparsable_date_leaves_periods_null() {
    let row = UsageRow {
        date: "05/03/2024".to_owned(),
        ..usage()
    };
    assert_eq!(
        column(row.clone(), "ChargePeriodStart"),
        ColumnValues::Text(vec![None])
    );
    assert_eq!(
        column(row, "BillingPeriodEnd"),
        ColumnValues::Text(vec![None])
    );
}

#[rstest]
#[case::ties_round_to_even(0.000_037_5_f64, Decimal::new(38, 6))]
#[case::ties_round_down_to_even(0.000_012_5_f64, Decimal::new(12, 6))]
#[case::below_half(0.000_000_4_f64, Decimal::ZERO)]
#[case::largest_fit(999_999_999_999.5_f64, Decimal::new(999_999_999_999_500_000, 6))]
#[case::largest_negative(-999_999_999_999.5_f64, Decimal::new(-999_999_999_999_500_000, 6))]
fn spend_rounds_to_six_places(#[case] spend: f64, #[case] expected: Decimal) {
    let row = UsageRow {
        spend: Decimal::try_from(spend).unwrap(),
        ..usage()
    };
    assert_eq!(
        column(row, "BilledCost"),
        ColumnValues::Decimal(vec![Some(expected)])
    );
}

#[rstest]
#[case::spend_overflow(1e12_f64, 0_i64)]
#[case::negative_spend_overflow(-1e12_f64, 0_i64)]
#[case::requests_overflow(0.0_f64, 1_000_000_000_000_i64)]
fn values_past_decimal_18_6_fail(#[case] spend: f64, #[case] api_requests: i64) {
    let row = UsageRow {
        spend: Decimal::try_from(spend).unwrap(),
        api_requests,
        ..usage()
    };
    let error = transform(&[row]).unwrap_err();
    assert!(
        matches!(error, TransformError::Overflow { .. }),
        "{error:?}"
    );
}
