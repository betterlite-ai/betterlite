use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use crate::auth_limit_inputs::estimates::{
    ESTIMATE_PER_MODEL_FIELD, ESTIMATED_OUTPUT_TOKENS_FIELD, get_estimated_output_tokens,
};
use crate::auth_limit_inputs::tests::support;

fn global_metadata(value: JsonData) -> JsonData {
    support::object(&[(ESTIMATED_OUTPUT_TOKENS_FIELD, value)])
}

fn per_model_metadata(value: JsonData) -> JsonData {
    support::object(&[(ESTIMATE_PER_MODEL_FIELD, value)])
}

fn estimate(
    auth: &crate::resolved_key::auth::UserAPIKeyAuth,
    model: Option<&str>,
) -> (Option<i64>, Vec<String>) {
    support::collect_warnings(|| get_estimated_output_tokens(auth, model))
}

#[rstest]
#[case::integer(JsonData::Number(serde_json::Number::from(12)), 12)]
#[case::integer_string(support::text("12"), 12)]
#[case::padded_string(support::text(" 12 "), 12)]
#[case::zero_fraction(support::text("12.0"), 12)]
#[case::underscored(support::text("1_0"), 10)]
#[case::whole_float(support::float(12.0), 12)]
#[case::signed_string(support::text("+12"), 12)]
#[case::underscored_with_zero_fraction(support::text("1_0.00"), 10)]
fn positive_int_coercion(value: JsonData, expected: i64) {
    let auth = support::auth(&[("metadata", global_metadata(value))]);
    let (estimate, warnings) = estimate(&auth, None);
    assert_eq!(estimate, Some(expected));
    assert!(warnings.is_empty());
}

#[rstest]
#[case::zero(JsonData::Number(serde_json::Number::from(0)))]
#[case::negative(JsonData::Number(serde_json::Number::from(-5)))]
#[case::bool_false(JsonData::Bool(false))]
fn zero_negative_invalid_warn(value: JsonData) {
    let auth = support::auth(&[("metadata", global_metadata(value))]);
    let (estimate, warnings) = estimate(&auth, None);
    assert_eq!(estimate, None);
    assert_eq!(warnings, vec![ESTIMATED_OUTPUT_TOKENS_FIELD.to_owned()]);
}

#[rstest]
#[case::text(support::text("abc"))]
#[case::fraction(support::float(1.5))]
#[case::padded_fraction(support::text("1.5"))]
#[case::trailing_dot(support::text("12."))]
#[case::leading_underscore(support::text("_12"))]
#[case::trailing_underscore(support::text("12_"))]
#[case::doubled_underscore(support::text("1__2"))]
#[case::underscore_in_fraction(support::text("12.0_0"))]
#[case::sign_only(support::text("+"))]
#[case::list(JsonData::Array(Vec::new()))]
#[case::map(JsonData::Object(std::collections::BTreeMap::new()))]
fn malformed_scalar_warn(value: JsonData) {
    let auth = support::auth(&[("metadata", global_metadata(value))]);
    let (estimate, warnings) = estimate(&auth, None);
    assert_eq!(estimate, None);
    assert_eq!(warnings, vec![ESTIMATED_OUTPUT_TOKENS_FIELD.to_owned()]);
}

#[test]
fn any_bad_map_entry_drops_map() {
    let auth = support::auth(&[(
        "metadata",
        support::object(&[
            (
                ESTIMATE_PER_MODEL_FIELD,
                support::object(&[
                    ("gpt-4", support::number(5)),
                    ("gpt-3", support::text("bad")),
                ]),
            ),
            (ESTIMATED_OUTPUT_TOKENS_FIELD, support::number(2002)),
        ]),
    )]);
    let (estimate, warnings) = estimate(&auth, Some("gpt-4"));
    assert_eq!(estimate, Some(2002));
    assert_eq!(warnings, vec![ESTIMATE_PER_MODEL_FIELD.to_owned()]);
}

#[test]
fn map_before_global() {
    let auth = support::auth(&[(
        "metadata",
        support::object(&[
            (
                ESTIMATE_PER_MODEL_FIELD,
                support::number_object(&[("gpt-4", 3001)]),
            ),
            (ESTIMATED_OUTPUT_TOKENS_FIELD, support::number(2002)),
        ]),
    )]);
    let (estimate, warnings) = estimate(&auth, Some("gpt-4"));
    assert_eq!(estimate, Some(3001));
    assert!(warnings.is_empty());
}

#[test]
fn no_model_skips_map_validation() {
    let auth = support::auth(&[(
        "metadata",
        support::object(&[
            (
                ESTIMATE_PER_MODEL_FIELD,
                support::object(&[("gpt-4", support::text("bad"))]),
            ),
            (ESTIMATED_OUTPUT_TOKENS_FIELD, support::number(2002)),
        ]),
    )]);
    let (estimate, warnings) = estimate(&auth, None);
    assert_eq!(estimate, Some(2002));
    assert!(warnings.is_empty());
}

#[test]
fn malformed_fields_independent() {
    let broken_map = support::auth(&[(
        "metadata",
        support::object(&[
            (
                ESTIMATE_PER_MODEL_FIELD,
                support::object(&[("gpt-4", support::text("bad"))]),
            ),
            (ESTIMATED_OUTPUT_TOKENS_FIELD, support::number(2002)),
        ]),
    )]);
    let (estimate, warnings) = estimate(&broken_map, Some("gpt-4"));
    assert_eq!(estimate, Some(2002));
    assert_eq!(warnings, vec![ESTIMATE_PER_MODEL_FIELD.to_owned()]);
    let broken_global = support::auth(&[(
        "metadata",
        support::object(&[
            (
                ESTIMATE_PER_MODEL_FIELD,
                support::number_object(&[("gpt-4", 3001)]),
            ),
            (ESTIMATED_OUTPUT_TOKENS_FIELD, support::number(-5)),
        ]),
    )]);
    let (estimate, warnings) = estimate(&broken_global, Some("gpt-4"));
    assert_eq!(estimate, Some(3001));
    assert!(warnings.is_empty());
}

#[test]
fn key_global_before_team_map() {
    let auth = support::auth(&[
        ("metadata", global_metadata(support::number(2002))),
        (
            "team_metadata",
            per_model_metadata(support::number_object(&[("gpt-4", 1503)])),
        ),
    ]);
    let (estimate, warnings) = estimate(&auth, Some("gpt-4"));
    assert_eq!(estimate, Some(2002));
    assert!(warnings.is_empty());
}

#[test]
fn missing_estimate_none() {
    let auth = support::auth(&[]);
    let (estimate, warnings) = estimate(&auth, Some("gpt-4"));
    assert_eq!(estimate, None);
    assert!(warnings.is_empty());
    let unrelated = support::auth(&[(
        "metadata",
        support::object(&[("unrelated", support::text("value"))]),
    )]);
    let (estimate, warnings) = estimate(&unrelated, Some("gpt-4"));
    assert_eq!(estimate, None);
    assert!(warnings.is_empty());
}
