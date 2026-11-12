#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;

use super::{SettingsError, endpoint_url, resolve};

fn params(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn reads_the_table_name_and_region() {
    let settings = resolve(
        &params(&[("dynamodb_table_name", "logs")]),
        &environment(&[("AWS_REGION_NAME", "us-east-1")]),
    )
    .expect("the settings resolve");
    assert_eq!(settings.table_name, "logs");
    assert_eq!(settings.region_name, "us-east-1");
}

#[test]
fn requires_the_region_variable() {
    let error = resolve(
        &params(&[("dynamodb_table_name", "logs")]),
        &environment(&[]),
    )
    .expect_err("the region is missing");
    assert!(matches!(error, SettingsError::MissingRegion));
    assert!(error.to_string().contains("AWS_REGION_NAME"));
}

#[test]
fn an_empty_region_variable_counts_as_unset() {
    let error = resolve(
        &params(&[("dynamodb_table_name", "logs")]),
        &environment(&[("AWS_REGION_NAME", "")]),
    )
    .expect_err("the region is missing");
    assert!(matches!(error, SettingsError::MissingRegion));
}

#[test]
fn requires_the_table_name_with_the_litellm_message() {
    let error = resolve(
        &params(&[]),
        &environment(&[("AWS_REGION_NAME", "us-east-1")]),
    )
    .expect_err("the table is missing");
    assert!(matches!(error, SettingsError::MissingTableName));
    assert_eq!(
        error.to_string(),
        "LiteLLM Error, trying to use DynamoDB but not table name passed. Create a table and set `litellm.dynamodb_table_name=<your-table>`"
    );
}

#[test]
fn the_region_is_refused_before_the_table_name() {
    let error = resolve(&params(&[]), &environment(&[])).expect_err("both are missing");
    assert!(matches!(error, SettingsError::MissingRegion));
}

#[test]
fn rejects_an_unknown_setting() {
    let error = resolve(
        &params(&[("dynamodb_table_name", "logs"), ("dynamodb_typo", "1")]),
        &environment(&[("AWS_REGION_NAME", "us-east-1")]),
    )
    .expect_err("the setting is unknown");
    assert!(matches!(error, SettingsError::UnknownKey { name } if name == "dynamodb_typo"));
}

#[test]
fn builds_the_partition_aware_endpoint() {
    assert_eq!(
        endpoint_url("us-east-1"),
        "https://dynamodb.us-east-1.amazonaws.com/"
    );
    assert_eq!(
        endpoint_url("cn-north-1"),
        "https://dynamodb.cn-north-1.amazonaws.com.cn/"
    );
    assert_eq!(
        endpoint_url("us-gov-west-1"),
        "https://dynamodb.us-gov-west-1.amazonaws.com/"
    );
}

#[test]
fn an_empty_table_name_is_used_as_given() {
    let settings = resolve(
        &params(&[("dynamodb_table_name", "")]),
        &environment(&[("AWS_REGION_NAME", "us-east-1")]),
    )
    .expect("the settings resolve");
    assert_eq!(settings.table_name, "");
}
