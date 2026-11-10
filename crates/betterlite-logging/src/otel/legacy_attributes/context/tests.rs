#![cfg(test)]

use rstest::rstest;

use super::truthiness;
use crate::otel::model::utils::coercion::OtelModelValue;

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

#[rstest]
#[case::null("null", false)]
#[case::true_value("true", true)]
#[case::false_value("false", false)]
#[case::zero("0", false)]
#[case::non_zero("1", true)]
#[case::zero_float("0.0", false)]
#[case::empty_text("\"\"", false)]
#[case::zero_text("\"0\"", true)]
#[case::text("\"x\"", true)]
#[case::empty_list("[]", false)]
#[case::list("[1]", true)]
#[case::empty_object("{}", false)]
#[case::object("{\"a\": 1}", true)]
fn truthiness_matches_python_truthiness(#[case] raw: &str, #[case] expected: bool) {
    assert_eq!(truthiness(&value(raw)), expected);
}
