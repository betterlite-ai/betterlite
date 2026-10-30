#![cfg(test)]

use super::*;

#[rstest]
#[case::numeric("{\"error\":{\"code\":429}}", Some(429))]
#[case::string("{\"error\":{\"code\":\"429\"}}", Some(429))]
#[case::malformed("not-json", None)]
#[case::wrong_shape("{\"code\":429}", None)]
#[case::whole_float("{\"error\":{\"code\":429.0}}", Some(429))]
#[case::truncates_toward_zero("{\"error\":{\"code\":429.9}}", Some(429))]
#[case::negative_truncates_toward_zero("{\"error\":{\"code\":-429.9}}", Some(-429))]
#[case::out_of_range_float("{\"error\":{\"code\":1e308}}", None)]
#[case::bool_true("{\"error\":{\"code\":true}}", Some(1))]
#[case::bool_false("{\"error\":{\"code\":false}}", Some(0))]
#[case::array("{\"error\":{\"code\":[429]}}", None)]
#[case::object("{\"error\":{\"code\":{\"inner\":429}}}", None)]
fn error_facts_behavior_tests_behavior(#[case] body: &str, #[case] expected: Option<i64>) {
    assert_eq!(get_body_error_code(body), expected);
}
