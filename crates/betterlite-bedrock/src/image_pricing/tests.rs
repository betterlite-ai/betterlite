#![cfg(test)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use rust_decimal::Decimal;

use super::error::ImagePricingError;
use super::{ImageCostRequest, image_cost};
use crate::image_models::params::{Field, Params};

fn decimal(value: &str) -> Decimal {
    Decimal::from_str(value).expect("a fixture decimal parses")
}

fn record(pairs: &[(&str, &str)]) -> PriceRecord {
    pairs
        .iter()
        .map(|(key, value)| {
            (
                (*key).to_owned(),
                serde_json::from_str(value).expect("a fixture scalar parses"),
            )
        })
        .collect()
}

struct CatalogLookup {
    entries: BTreeMap<String, PriceRecord>,
    last_key: Cell<Option<String>>,
}

const CROSS_REGION_PREFIXES: [&str; 7] =
    ["global.", "us.", "eu.", "apac.", "jp.", "au.", "us-gov."];

// The effective lookup strips a Bedrock cross-region prefix and matches the base model case-insensitively.
fn bedrock_base_model(model: &str) -> &str {
    for prefix in CROSS_REGION_PREFIXES {
        if let Some(stripped) = model.strip_prefix(prefix) {
            return stripped;
        }
    }
    model
}

impl CatalogLookup {
    fn new(entries: &[(&str, PriceRecord)]) -> Self {
        Self {
            entries: entries
                .iter()
                .map(|(key, entry)| (key.to_lowercase(), entry.clone()))
                .collect(),
            last_key: Cell::new(None),
        }
    }

    fn last_key(&self) -> Option<String> {
        self.last_key.take()
    }
}

impl PriceLookup for CatalogLookup {
    fn effective_model_info(
        &self,
        model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.last_key.set(Some(model.to_owned()));
        self.entries
            .get(&bedrock_base_model(model).to_lowercase())
            .cloned()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

fn params(steps: Field) -> Params {
    let mut params = Params::new();
    params.insert("steps".to_owned(), steps);
    params
}

fn cost(
    model: &str,
    image_count: i64,
    size: Option<&str>,
    optional_params: Option<&Params>,
    lookup: &CatalogLookup,
) -> Result<Decimal, ImagePricingError> {
    let request = ImageCostRequest {
        model,
        image_count,
        size,
        optional_params,
    };
    image_cost(&request, lookup)
}

#[test]
fn titan_bills_the_returned_images_at_its_unqualified_price() {
    let lookup = CatalogLookup::new(&[(
        "amazon.titan-image-generator-v1",
        record(&[("output_cost_per_image", "0.06")]),
    )]);
    let result = cost("amazon.titan-image-generator-v1", 2, None, None, &lookup)
        .expect("titan pricing succeeds");
    assert_eq!(result, decimal("0.12"));
    assert_eq!(
        lookup.last_key().as_deref(),
        Some("amazon.titan-image-generator-v1")
    );
}

#[test]
fn titan_bills_zero_for_zero_images() {
    let lookup = CatalogLookup::new(&[(
        "amazon.titan-image-generator-v1",
        record(&[("output_cost_per_image", "0.008")]),
    )]);
    let result = cost("amazon.titan-image-generator-v1", 0, None, None, &lookup)
        .expect("titan pricing succeeds");
    assert_eq!(result, Decimal::ZERO);
}

#[test]
fn titan_fails_for_an_unknown_model_before_the_zero_image_result() {
    let lookup = CatalogLookup::new(&[]);
    let result = cost("amazon.titan-image-generator-v1", 0, None, None, &lookup);
    assert!(matches!(
        result,
        Err(ImagePricingError::UnknownModel { .. })
    ));
}

#[test]
fn titan_bills_zero_when_the_price_is_missing() {
    let lookup = CatalogLookup::new(&[(
        "amazon.titan-image-generator-v1",
        record(&[("mode", "\"image_generation\"")]),
    )]);
    let result = cost("amazon.titan-image-generator-v1", 3, None, None, &lookup)
        .expect("titan pricing succeeds");
    assert_eq!(result, Decimal::ZERO);
}

#[test]
fn stability1_builds_the_size_step_and_model_key() {
    let lookup = CatalogLookup::new(&[(
        "512-x-512/max-steps/stability.stable-diffusion-xl-v1",
        record(&[("output_cost_per_image", "0.018")]),
    )]);
    let steps = params(Field::Int(51));
    let result = cost(
        "stability.stable-diffusion-xl-v1",
        2,
        Some("512-x-512"),
        Some(&steps),
        &lookup,
    )
    .expect("stability 1 pricing succeeds");
    assert_eq!(result, decimal("0.036"));
    assert_eq!(
        lookup.last_key().as_deref(),
        Some("512-x-512/max-steps/stability.stable-diffusion-xl-v1")
    );
}

#[test]
fn stability1_defaults_the_size_and_the_step_bucket() {
    let lookup = CatalogLookup::new(&[(
        "1024-x-1024/50-steps/stability.stable-diffusion-xl-v0",
        record(&[("output_cost_per_image", "0.04")]),
    )]);
    let result = cost("stability.stable-diffusion-xl-v0", 3, None, None, &lookup)
        .expect("stability 1 pricing succeeds");
    assert_eq!(result, decimal("0.12"));
    assert_eq!(
        lookup.last_key().as_deref(),
        Some("1024-x-1024/50-steps/stability.stable-diffusion-xl-v0")
    );
}

#[test]
fn stability1_keeps_the_supplied_size_spelling() {
    let lookup = CatalogLookup::new(&[(
        "1024x1024/50-steps/stability.stable-diffusion-xl-v0",
        record(&[("output_cost_per_image", "0.04")]),
    )]);
    let result = cost(
        "stability.stable-diffusion-xl-v0",
        1,
        Some("1024x1024"),
        None,
        &lookup,
    )
    .expect("stability 1 pricing succeeds");
    assert_eq!(result, decimal("0.04"));
    assert_eq!(
        lookup.last_key().as_deref(),
        Some("1024x1024/50-steps/stability.stable-diffusion-xl-v0")
    );
}

#[test]
fn stability1_matches_the_size_key_case_insensitively() {
    let lookup = CatalogLookup::new(&[(
        "1024-x-1024/50-steps/stability.stable-diffusion-xl-v1",
        record(&[("output_cost_per_image", "0.04")]),
    )]);
    let result = cost(
        "stability.stable-diffusion-xl-v1",
        1,
        Some("1024-X-1024"),
        None,
        &lookup,
    )
    .expect("stability 1 pricing succeeds");
    assert_eq!(result, decimal("0.04"));
}

#[test]
fn stability1_treats_an_empty_size_as_absent() {
    let lookup = CatalogLookup::new(&[(
        "1024-x-1024/50-steps/stability.stable-diffusion-xl-v0",
        record(&[("output_cost_per_image", "0.04")]),
    )]);
    let result = cost(
        "stability.stable-diffusion-xl-v0",
        1,
        Some(""),
        None,
        &lookup,
    )
    .expect("stability 1 pricing succeeds");
    assert_eq!(result, decimal("0.04"));
}

#[rstest::rstest]
#[case::at_fifty(Field::Int(50), "50-steps")]
#[case::above_fifty(Field::Int(51), "max-steps")]
#[case::negative(Field::Int(-5), "50-steps")]
#[case::float_above(Field::Float(50.5), "max-steps")]
#[case::float_at(Field::Float(50.0), "50-steps")]
#[case::boolean(Field::Bool(true), "50-steps")]
fn stability1_buckets_steps_around_fifty(#[case] steps: Field, #[case] bucket: &str) {
    let key = format!("1024-x-1024/{bucket}/stability.stable-diffusion-xl-v0");
    let lookup =
        CatalogLookup::new(&[(key.as_str(), record(&[("output_cost_per_image", "0.04")]))]);
    let params = params(steps);
    let result = cost(
        "stability.stable-diffusion-xl-v0",
        1,
        None,
        Some(&params),
        &lookup,
    )
    .expect("stability 1 pricing succeeds");
    assert_eq!(result, decimal("0.04"));
    assert_eq!(lookup.last_key().as_deref(), Some(key.as_str()));
}

#[rstest::rstest]
#[case::null(Field::Null)]
#[case::text(Field::Text("60".to_owned()))]
#[case::list(Field::List(Vec::new()))]
#[case::mapping(Field::Mapping(Params::new()))]
fn stability1_rejects_a_non_numeric_steps_value(#[case] steps: Field) {
    let lookup = CatalogLookup::new(&[]);
    let params = params(steps);
    let result = cost(
        "stability.stable-diffusion-xl-v0",
        1,
        None,
        Some(&params),
        &lookup,
    );
    assert!(matches!(result, Err(ImagePricingError::InvalidSteps)));
    assert_eq!(lookup.last_key(), None);
}

#[test]
fn stability1_fails_for_an_unknown_key_before_the_zero_image_result() {
    let lookup = CatalogLookup::new(&[]);
    let result = cost("stability.stable-diffusion-xl-v0", 0, None, None, &lookup);
    assert!(matches!(
        result,
        Err(ImagePricingError::UnknownModel { .. })
    ));
}

#[test]
fn stability3_bills_its_flat_model_price() {
    let lookup = CatalogLookup::new(&[(
        "stability.sd3-large-v1:0",
        record(&[("output_cost_per_image", "0.08")]),
    )]);
    let result = cost("stability.sd3-large-v1:0", 3, None, None, &lookup)
        .expect("stability 3 pricing succeeds");
    assert_eq!(result, decimal("0.24"));
    assert_eq!(
        lookup.last_key().as_deref(),
        Some("stability.sd3-large-v1:0")
    );
}

#[test]
fn nova_bills_its_flat_model_price() {
    let lookup = CatalogLookup::new(&[(
        "amazon.nova-canvas-v1:0",
        record(&[("output_cost_per_image", "0.06")]),
    )]);
    let result =
        cost("amazon.nova-canvas-v1:0", 2, None, None, &lookup).expect("nova pricing succeeds");
    assert_eq!(result, decimal("0.12"));
    assert_eq!(
        lookup.last_key().as_deref(),
        Some("amazon.nova-canvas-v1:0")
    );
}

#[test]
fn nova_resolves_a_region_prefixed_model() {
    let lookup = CatalogLookup::new(&[(
        "amazon.nova-canvas-v1:0",
        record(&[("output_cost_per_image", "0.06")]),
    )]);
    let result = cost("eu.amazon.nova-canvas-v1:0", 1, None, None, &lookup)
        .expect("region-prefixed nova pricing succeeds");
    assert_eq!(result, decimal("0.06"));
    assert_eq!(
        lookup.last_key().as_deref(),
        Some("eu.amazon.nova-canvas-v1:0")
    );
}

#[test]
fn stability3_resolves_a_region_prefixed_model() {
    let lookup = CatalogLookup::new(&[(
        "stability.sd3-large-v1:0",
        record(&[("output_cost_per_image", "0.08")]),
    )]);
    let result = cost("eu.stability.sd3-large-v1:0", 1, None, None, &lookup)
        .expect("region-prefixed stability 3 pricing succeeds");
    assert_eq!(result, decimal("0.08"));
    assert_eq!(
        lookup.last_key().as_deref(),
        Some("eu.stability.sd3-large-v1:0")
    );
}

#[test]
fn stability3_fails_for_an_unknown_model_before_the_zero_image_result() {
    let lookup = CatalogLookup::new(&[]);
    let result = cost("stability.sd3-large-v1:0", 0, None, None, &lookup);
    assert!(matches!(
        result,
        Err(ImagePricingError::UnknownModel { .. })
    ));
}

#[test]
fn stability3_bills_zero_for_zero_images() {
    let lookup = CatalogLookup::new(&[(
        "stability.sd3-large-v1:0",
        record(&[("output_cost_per_image", "0.08")]),
    )]);
    let result = cost("stability.sd3-large-v1:0", 0, None, None, &lookup)
        .expect("stability 3 pricing succeeds");
    assert_eq!(result, Decimal::ZERO);
}

#[test]
fn an_unrecognized_model_uses_the_stability1_key_shape() {
    let lookup = CatalogLookup::new(&[(
        "1024-x-1024/50-steps/custom.image-model",
        record(&[("output_cost_per_image", "0.04")]),
    )]);
    let result = cost("custom.image-model", 1, None, None, &lookup)
        .expect("the default family prices like stability 1");
    assert_eq!(result, decimal("0.04"));
    assert_eq!(
        lookup.last_key().as_deref(),
        Some("1024-x-1024/50-steps/custom.image-model")
    );
}

#[test]
fn multiplication_overflow_is_a_typed_failure() {
    let huge = Decimal::MAX.to_string();
    let lookup = CatalogLookup::new(&[(
        "amazon.nova-canvas-v1:0",
        record(&[("output_cost_per_image", huge.as_str())]),
    )]);
    let result = cost("amazon.nova-canvas-v1:0", 2, None, None, &lookup);
    assert!(matches!(result, Err(ImagePricingError::Overflow)));
}

#[test]
fn a_malformed_price_is_a_typed_price_failure() {
    let lookup = CatalogLookup::new(&[(
        "amazon.nova-canvas-v1:0",
        record(&[("output_cost_per_image", "\"nope\"")]),
    )]);
    let result = cost("amazon.nova-canvas-v1:0", 1, None, None, &lookup);
    assert!(matches!(result, Err(ImagePricingError::Price(_))));
}
