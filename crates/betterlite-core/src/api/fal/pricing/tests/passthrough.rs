use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use super::super::{FalPricingError, fal_ai_passthrough_cost};
use super::{Json, decimal, integer, lookup_from, object, text};

fn passthrough_pass_cost(body: &IndexMap<String, Json>) -> Option<Decimal> {
    let lookup = lookup_from(&[(
        "fal_ai/fal-ai/pass",
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0.5, "output_cost_per_image_1024": 0.25}"#,
    )]);
    fal_ai_passthrough_cost("fal-ai/pass", body, &lookup).expect("the entry prices the request")
}

#[test]
fn a_passthrough_resolution_price_counts_the_requested_images() {
    let body = object(&[("resolution", integer(1024)), ("num_images", integer(3))]);
    assert_eq!(passthrough_pass_cost(&body), Some(decimal("0.75")));
}

#[test]
fn a_passthrough_flat_price_counts_the_requested_images() {
    let body = object(&[("num_images", integer(3))]);
    assert_eq!(passthrough_pass_cost(&body), Some(decimal("1.5")));
}

#[test]
fn a_named_passthrough_resolution_price_counts_the_requested_images() {
    let body = object(&[("resolution", text("1024")), ("num_images", integer(3))]);
    assert_eq!(passthrough_pass_cost(&body), Some(decimal("0.75")));
}

#[rstest]
#[case::boolean("true")]
#[case::zero("0")]
#[case::negative("-1")]
#[case::float("2.0")]
#[case::text(r#""4""#)]
#[case::null("null")]
#[case::list("[]")]
fn a_count_that_is_not_a_positive_exact_integer_uses_one(#[case] raw_count: &str) {
    let count: Json = serde_json::from_str(raw_count).expect("the case is a JSON value");
    let body = object(&[("num_images", count)]);
    assert_eq!(passthrough_pass_cost(&body), Some(decimal("0.5")));
}

#[test]
fn a_positive_exact_integer_count_multiplies_the_price() {
    let body = object(&[("num_images", integer(4))]);
    assert_eq!(passthrough_pass_cost(&body), Some(decimal("2")));
}

#[test]
fn passthrough_cost_distinguishes_an_unavailable_price_from_zero() {
    let lookup = lookup_from(&[
        ("fal_ai/fal-ai/missing", r#"{"litellm_provider": "fal_ai"}"#),
        (
            "fal_ai/fal-ai/zero",
            r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 0}"#,
        ),
    ]);
    let body = object(&[]);
    assert_eq!(
        fal_ai_passthrough_cost("fal-ai/missing", &body, &lookup)
            .expect("a missing price is not an error"),
        None
    );
    assert_eq!(
        fal_ai_passthrough_cost("fal-ai/unknown", &body, &lookup)
            .expect("a missing entry is not an error"),
        None
    );
    assert_eq!(
        fal_ai_passthrough_cost("fal-ai/zero", &body, &lookup).expect("the zero price is a price"),
        Some(Decimal::ZERO)
    );
    let flag = object(&[("resolution", Json::Bool(true))]);
    assert_eq!(
        fal_ai_passthrough_cost("fal-ai/zero", &flag, &lookup)
            .expect("a boolean resolution is ignored"),
        Some(Decimal::ZERO)
    );
}

#[test]
fn a_boolean_catalog_price_reads_as_one() {
    let lookup = lookup_from(&[(
        "fal_ai/fal-ai/flag",
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image": true}"#,
    )]);
    let body = object(&[]);
    assert_eq!(
        fal_ai_passthrough_cost("fal-ai/flag", &body, &lookup)
            .expect("the boolean price is a price"),
        Some(Decimal::ONE)
    );
}

#[test]
fn passthrough_cost_reports_an_overflowing_product() {
    let lookup = lookup_from(&[(
        "fal_ai/fal-ai/huge",
        r#"{"litellm_provider": "fal_ai", "output_cost_per_image": 79228162514264337593543950335}"#,
    )]);
    let body = object(&[("num_images", integer(2))]);
    assert!(matches!(
        fal_ai_passthrough_cost("fal-ai/huge", &body, &lookup),
        Err(FalPricingError::Overflow)
    ));
}
