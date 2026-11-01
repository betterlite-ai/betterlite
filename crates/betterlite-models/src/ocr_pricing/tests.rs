#![cfg(test)]

use super::{
    OcrBatchUsage, OcrBilling, OcrCostError, OcrPricingField, OcrResponseKind, OcrUsage,
    first_ocr_price, first_price, layered_ocr_pricing, ocr_batch_cost, ocr_cost, single_log_line,
};
use crate::price_lookup::{PriceLookup, PriceRecord};
use crate::tiered_pricing::Scalar;
use rstest::rstest;
use rust_decimal::Decimal;
use std::cell::Cell;

fn number(value: i64) -> Scalar {
    Scalar::Number(serde_json::Number::from(value))
}

fn record(pairs: &[(&str, Scalar)]) -> PriceRecord {
    pairs
        .iter()
        .cloned()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn dec(value: &str) -> Decimal {
    Decimal::from_str_exact(value).expect("test literal parses")
}

fn usage(credits: Option<i64>, pages: Option<i64>, annotation_pages: i64) -> OcrUsage {
    OcrUsage {
        credits: credits.map(Decimal::from),
        pages_processed: pages,
        pages_processed_annotation: annotation_pages,
    }
}

fn billing(usage: OcrUsage) -> OcrBilling {
    OcrBilling {
        kind: OcrResponseKind::Ocr,
        usage: Some(usage),
    }
}

fn deployment(
    ocr_cost_per_page: Option<i64>,
    ocr_cost_per_credit: Option<i64>,
    annotation_cost_per_page: Option<i64>,
) -> PriceRecord {
    let mut pairs = Vec::new();
    if let Some(value) = ocr_cost_per_page {
        pairs.push(("ocr_cost_per_page", number(value)));
    }
    if let Some(value) = ocr_cost_per_credit {
        pairs.push(("ocr_cost_per_credit", number(value)));
    }
    if let Some(value) = annotation_cost_per_page {
        pairs.push(("annotation_cost_per_page", number(value)));
    }
    record(&pairs)
}

struct NullLookup;

impl PriceLookup for NullLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }
}

struct StaticLookup(Option<PriceRecord>);

impl PriceLookup for StaticLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.0.clone()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }
}

// Counts effective_model_info calls so a test can prove an already-priced family is never fetched.
struct CountingLookup {
    answer: Option<PriceRecord>,
    calls: Cell<u32>,
}

impl CountingLookup {
    fn new(answer: Option<PriceRecord>) -> Self {
        Self {
            answer,
            calls: Cell::new(0),
        }
    }
}

impl PriceLookup for CountingLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.calls.set(self.calls.get().saturating_add(1));
        self.answer.clone()
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }
}

#[rstest]
#[case::string_is_skipped_in_favor_of_a_later_numeric_zero(
    &[Some(record(&[("ocr_cost_per_page", Scalar::Text("5".to_owned()))])), Some(record(&[("ocr_cost_per_page", number(0))]))],
    Some(Decimal::ZERO)
)]
#[case::boolean_true_counts_as_one(
    &[Some(record(&[("ocr_cost_per_page", Scalar::Bool(true))]))],
    Some(Decimal::ONE)
)]
#[case::missing_field_falls_through_every_source(&[Some(record(&[])), None], None)]
fn first_ocr_price_layers_numeric_sources_and_skips_strings(
    #[case] sources: &[Option<PriceRecord>],
    #[case] expected: Option<Decimal>,
) {
    let refs: Vec<Option<&PriceRecord>> = sources.iter().map(Option::as_ref).collect();
    assert_eq!(
        first_ocr_price(OcrPricingField::OcrCostPerPage, &refs).unwrap(),
        expected
    );
}

#[test]
fn layered_ocr_pricing_resolves_each_field_independently() {
    let deployment_record = deployment(Some(1), None, Some(2));
    let catalog_record = record(&[("ocr_cost_per_credit", number(3))]);
    let sources = [Some(&deployment_record), Some(&catalog_record)];

    let pricing = layered_ocr_pricing(&sources).unwrap();

    assert_eq!(pricing.ocr_cost_per_page, Some(Decimal::ONE));
    assert_eq!(pricing.annotation_cost_per_page, Some(Decimal::from(2_i32)));
    assert_eq!(pricing.ocr_cost_per_credit, Some(Decimal::from(3_i32)));
}

#[rstest]
#[case::primary_key_found(&[("ocr_cost_per_page_batches", number(9))], Some(Decimal::from(9_i32)))]
#[case::falls_back_to_second_key(&[("ocr_cost_per_page", number(4))], Some(Decimal::from(4_i32)))]
#[case::absent_from_every_key(&[], None)]
fn first_price_checks_each_key_in_order(
    #[case] pairs: &[(&str, Scalar)],
    #[case] expected: Option<Decimal>,
) {
    let source = record(pairs);
    assert_eq!(
        first_price(
            Some(&source),
            &["ocr_cost_per_page_batches", "ocr_cost_per_page"]
        )
        .unwrap(),
        expected
    );
}

#[test]
fn single_log_line_strips_cr_and_lf_and_spells_an_absent_value_as_none() {
    assert_eq!(single_log_line(Some("az\r\nure")), "azure");
    assert_eq!(single_log_line(None), "None");
}

#[test]
fn validate_rejects_a_response_that_is_not_ocr_shaped() {
    let billing = OcrBilling {
        kind: OcrResponseKind::Other,
        usage: None,
    };
    assert!(matches!(
        billing.validate(),
        Err(OcrCostError::WrongResponseKind)
    ));
}

#[test]
fn validate_rejects_an_ocr_response_with_no_usage() {
    let billing = OcrBilling {
        kind: OcrResponseKind::Ocr,
        usage: None,
    };
    assert!(matches!(
        billing.validate(),
        Err(OcrCostError::MissingUsage)
    ));
}

enum Expected {
    Cost(&'static str),
    MissingPages,
}

#[rstest]
#[case::credit_precedence_wins_even_at_zero_credits(
    usage(Some(0), Some(100), 0),
    deployment(Some(1), Some(5), None),
    Expected::Cost("0")
)]
#[case::credit_precedence_wins_even_at_zero_rate(
    usage(Some(3), None, 0),
    deployment(None, Some(0), None),
    Expected::Cost("0")
)]
#[case::annotation_rate_falls_back_to_page_rate(
    usage(None, Some(2), 3),
    deployment(Some(10), None, None),
    Expected::Cost("50")
)]
#[case::explicit_annotation_rate_used_when_present(
    usage(None, Some(2), 3),
    deployment(Some(10), None, Some(4)),
    Expected::Cost("32")
)]
#[case::missing_ordinary_pages_permitted_by_billable_annotation(
    usage(None, None, 5),
    deployment(Some(10), None, None),
    Expected::Cost("50")
)]
#[case::missing_pages_no_billable_annotation_credit_priced_warns_zero(
    usage(None, None, 0),
    deployment(Some(10), Some(1), None),
    Expected::Cost("0")
)]
#[case::missing_pages_no_billable_annotation_unpriced_warns_zero(
    usage(None, None, 0),
    deployment(None, None, None),
    Expected::Cost("0")
)]
#[case::missing_pages_no_billable_annotation_page_priced_errors(
    usage(None, None, 0),
    deployment(Some(10), None, None),
    Expected::MissingPages
)]
#[case::present_pages_no_rate_no_annotation_warns_zero(
    usage(None, Some(5), 0),
    deployment(None, None, None),
    Expected::Cost("0")
)]
#[case::negative_pages_retain_arithmetic_sign(
    usage(None, Some(-5), 0), deployment(Some(2), None, None), Expected::Cost("-10")
)]
fn ocr_cost_follows_credit_and_page_precedence(
    #[case] usage: OcrUsage,
    #[case] deployment: PriceRecord,
    #[case] expected: Expected,
) {
    let billing = billing(usage);
    let result = ocr_cost(
        "model",
        Some("provider"),
        &billing,
        Some(&deployment),
        &NullLookup,
    );
    match expected {
        Expected::Cost(value) => assert_eq!(result.unwrap(), dec(value)),
        Expected::MissingPages => {
            assert!(matches!(result, Err(OcrCostError::MissingPages { .. })));
        }
    }
}

#[test]
fn partial_override_falls_back_field_by_field_to_effective_prices() {
    let lookup = StaticLookup(Some(record(&[
        ("ocr_cost_per_page", number(3)),
        ("annotation_cost_per_page", number(7)),
    ])));
    let deployment_override = deployment(Some(100), None, None);
    let billing = billing(usage(None, Some(2), 1));

    let cost = ocr_cost("model", None, &billing, Some(&deployment_override), &lookup).unwrap();

    assert_eq!(cost, dec("207"));
}

#[test]
fn batch_cost_suppresses_lookup_when_zero_counts_need_no_family() {
    let lookup = CountingLookup::new(None);
    let usage = OcrBatchUsage {
        pages_processed: 0,
        pages_processed_annotation: 0,
    };

    let cost = ocr_batch_cost("model", None, &usage, None, &lookup).unwrap();

    assert_eq!(cost, Decimal::ZERO);
    assert_eq!(lookup.calls.get(), 0);
}

#[test]
fn batch_cost_retains_sign_and_suppresses_lookup_for_negative_counts() {
    let lookup = CountingLookup::new(None);
    let usage = OcrBatchUsage {
        pages_processed: -5,
        pages_processed_annotation: 0,
    };
    let deployment_info = deployment(Some(2), None, None);

    let cost = ocr_batch_cost("model", None, &usage, Some(&deployment_info), &lookup).unwrap();

    assert_eq!(cost, dec("-10"));
    assert_eq!(lookup.calls.get(), 0);
}

#[test]
fn batch_cost_resolves_page_and_annotation_families_independently() {
    let page_batch_record = record(&[("ocr_cost_per_page_batches", number(10))]);
    let lookup = CountingLookup::new(Some(record(&[("annotation_cost_per_page", number(4))])));
    let usage = OcrBatchUsage {
        pages_processed: 2,
        pages_processed_annotation: 3,
    };

    let cost = ocr_batch_cost("model", None, &usage, Some(&page_batch_record), &lookup).unwrap();

    assert_eq!(cost, dec("32"));
    assert_eq!(lookup.calls.get(), 1);
}

#[test]
fn batch_cost_prefers_deployment_synchronous_rate_over_public_batch_rate() {
    let deployment_sync_record = record(&[("ocr_cost_per_page", number(5))]);
    let lookup = CountingLookup::new(Some(record(&[("ocr_cost_per_page_batches", number(99))])));
    let usage = OcrBatchUsage {
        pages_processed: 2,
        pages_processed_annotation: 0,
    };

    let cost = ocr_batch_cost(
        "model",
        None,
        &usage,
        Some(&deployment_sync_record),
        &lookup,
    )
    .unwrap();

    assert_eq!(cost, dec("10"));
    assert_eq!(lookup.calls.get(), 0);
}

#[test]
fn batch_cost_still_fetches_public_annotation_price_when_deployment_only_prices_pages() {
    let deployment_page_record = record(&[("ocr_cost_per_page", number(5))]);
    let lookup = CountingLookup::new(Some(record(&[])));
    let usage = OcrBatchUsage {
        pages_processed: 0,
        pages_processed_annotation: 3,
    };

    let cost = ocr_batch_cost(
        "model",
        None,
        &usage,
        Some(&deployment_page_record),
        &lookup,
    )
    .unwrap();

    // No published annotation rate either, so annotation falls back to the resolved page rate.
    assert_eq!(cost, dec("15"));
    assert_eq!(lookup.calls.get(), 1);
}

#[test]
fn batch_cost_bills_only_the_families_that_remain_priced_after_a_failed_lookup() {
    let lookup = CountingLookup::new(None);
    let usage = OcrBatchUsage {
        pages_processed: 2,
        pages_processed_annotation: 0,
    };

    let cost = ocr_batch_cost("model", None, &usage, None, &lookup).unwrap();

    assert_eq!(cost, Decimal::ZERO);
    assert_eq!(lookup.calls.get(), 1);
}

// Proves docs/deviations/B1678.yaml: a product too large for Decimal is a typed error, not a rounded float.
#[test]
fn ocr_cost_rejects_an_unrepresentable_product_instead_of_rounding_through_a_float() {
    let max_rate = Scalar::Number(
        serde_json::from_str("79228162514264337593543950335").expect("Decimal::MAX parses"),
    );
    let deployment_override = record(&[("ocr_cost_per_page", max_rate)]);
    let billing = billing(usage(None, Some(2), 0));

    let result = ocr_cost(
        "model",
        None,
        &billing,
        Some(&deployment_override),
        &NullLookup,
    );

    assert!(matches!(result, Err(OcrCostError::Overflow)));
}

// docs/deviations/B1678.yaml: a price Decimal cannot hold exactly is a typed failure, not a skip to the next source.
#[test]
fn first_ocr_price_errors_on_an_unrepresentable_number_instead_of_falling_through() {
    let unrepresentable = Scalar::Number(
        serde_json::from_str("0.1234567890123456789012345678901").expect("JSON number parses"),
    );
    let bad_source = record(&[("ocr_cost_per_page", unrepresentable)]);
    let good_source = record(&[("ocr_cost_per_page", number(5))]);
    let sources = [Some(&bad_source), Some(&good_source)];

    let result = first_ocr_price(OcrPricingField::OcrCostPerPage, &sources);

    assert!(matches!(
        result,
        Err(OcrCostError::UnrepresentablePrice { .. })
    ));
}
