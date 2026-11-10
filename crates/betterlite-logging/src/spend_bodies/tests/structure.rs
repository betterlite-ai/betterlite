use betterlite_models::decisions::Json;

use super::support::value;
use crate::private_json::safe_json::DEFAULT_MAX_RECURSE_DEPTH;
use crate::python_json;
use crate::spend_bodies::support::{counts_as_present, safe_structure};

fn nest(depth: usize) -> Json {
    let mut value = Json::Text("leaf".to_owned());
    for _ in 0..depth {
        value = Json::List(vec![value]);
    }
    value
}

#[rstest::rstest]
#[case::null("null", false)]
#[case::true_value("true", true)]
#[case::false_value("false", false)]
#[case::zero("0", false)]
#[case::negative_zero("-0.0", false)]
#[case::one("1", true)]
#[case::negative_one("-1", true)]
#[case::empty_text("\"\"", false)]
#[case::text("\"x\"", true)]
#[case::empty_list("[]", false)]
#[case::list("[1]", true)]
#[case::empty_object("{}", false)]
#[case::object("{\"a\": 1}", true)]
fn counts_truthy_values_like_python(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(counts_as_present(&value(text)), expected);
}

#[test]
fn replaces_values_past_the_depth_bound_with_the_marker() {
    let within = python_json::dumps(&safe_structure(&nest(DEFAULT_MAX_RECURSE_DEPTH), 0));
    assert!(!within.contains("MaxDepthExceeded"));
    let beyond = python_json::dumps(&safe_structure(&nest(DEFAULT_MAX_RECURSE_DEPTH + 1), 0));
    assert!(beyond.contains("MaxDepthExceeded"));
}
