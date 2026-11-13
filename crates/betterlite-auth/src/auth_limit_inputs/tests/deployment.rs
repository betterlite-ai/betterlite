use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use crate::auth_limit_inputs::deployment::DeploymentLimitValue;
use crate::auth_limit_inputs::tests::support;

fn float_number(value: f64) -> Json {
    Json::Number(Number::from_f64(value).unwrap())
}

fn empty_object() -> Json {
    serde_json::from_str("{}").unwrap()
}

#[rstest]
#[case::integer(Json::Number(Number::from(-7)), DeploymentLimitValue::Integer(-7))]
#[case::unsigned(
    Json::Number(Number::from(u64::MAX)),
    DeploymentLimitValue::Unsigned(u64::MAX)
)]
#[case::float(float_number(2.5), DeploymentLimitValue::Float(2.5))]
#[case::boolean(Json::Bool(true), DeploymentLimitValue::Bool(true))]
#[case::text(Json::Text("12".to_owned()), DeploymentLimitValue::Text("12".to_owned()))]
#[case::null(Json::Null, DeploymentLimitValue::Unsupported)]
#[case::list(Json::List(Vec::new()), DeploymentLimitValue::Unsupported)]
#[case::object(empty_object(), DeploymentLimitValue::Unsupported)]
fn json_value_reads_as_int_input(#[case] value: Json, #[case] expected: DeploymentLimitValue) {
    assert_eq!(DeploymentLimitValue::from(&value), expected);
}

fn text_limit(text: &str) -> Option<BTreeMap<String, i64>> {
    let value = DeploymentLimitValue::Text(text.to_owned());
    let catalog = support::catalog("gpt-4", vec![support::deployment(Some(value), None)]);
    support::model_limits(&support::auth(&[]), Some("gpt-4"), Some(&catalog)).unwrap()
}

#[rstest]
#[case::underscored("1_000", 1000)]
#[case::padded(" 7 ", 7)]
#[case::signed_positive("+7", 7)]
#[case::signed_negative("-3", -3)]
fn text_limit_is_read_like_int(#[case] text: &str, #[case] expected: i64) {
    assert_eq!(
        text_limit(text),
        Some(support::limits(&[("gpt-4", expected)]))
    );
}

#[rstest]
#[case::leading_underscore("_12")]
#[case::trailing_underscore("12_")]
#[case::doubled_underscore("1__2")]
#[case::signed_leading_underscore("+_12")]
#[case::letters("abc")]
#[case::empty("")]
#[case::sign_only("-")]
#[case::fraction("1.5")]
fn malformed_text_limit_skipped(#[case] text: &str) {
    assert_eq!(text_limit(text), None);
}
