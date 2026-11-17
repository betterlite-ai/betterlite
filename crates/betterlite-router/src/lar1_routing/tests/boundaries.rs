use std::time::Duration;

use rstest::rstest;

use super::support::{
    Failing, Group, Pending, args, dec, selected, spec, strategy_with, typed_pool,
};
use crate::lar1_routing::metadata::{Lar1Metadata, MAX_METADATA_BYTES, MetadataError};
use crate::lar1_routing::strategy::{Lar1Strategy, Target, pick};
use crate::lar1_routing::thresholds::{ThresholdError, Thresholds};
use crate::router_core::error::RoutingError;

#[derive(Clone, Copy)]
enum Boundary {
    MetadataAtLimit,
    MetadataOverLimit,
    ConfidenceZero,
    ConfidenceOne,
    ConfidenceUnderZero,
    ConfidenceOverOne,
    MalformedMetadata,
    ThresholdOrder,
    ThresholdLowEqualsMedium,
    ThresholdMediumEqualsHigh,
    ThresholdZeroLow,
    ThresholdOneHigh,
    ThresholdJustInside,
    ClassificationEdge(&'static str, &'static str),
    PickEmpty,
    PickFirstMatch,
    PickFallback,
    PickSkipsUntyped,
    Cancelled,
    Pending,
}

#[rstest]
#[case::metadata_at_limit(Boundary::MetadataAtLimit)]
#[case::metadata_over_limit(Boundary::MetadataOverLimit)]
#[case::confidence_zero(Boundary::ConfidenceZero)]
#[case::confidence_one(Boundary::ConfidenceOne)]
#[case::confidence_under_zero(Boundary::ConfidenceUnderZero)]
#[case::confidence_over_one(Boundary::ConfidenceOverOne)]
#[case::malformed_metadata(Boundary::MalformedMetadata)]
#[case::threshold_order(Boundary::ThresholdOrder)]
#[case::threshold_low_equals_medium(Boundary::ThresholdLowEqualsMedium)]
#[case::threshold_medium_equals_high(Boundary::ThresholdMediumEqualsHigh)]
#[case::threshold_zero_low(Boundary::ThresholdZeroLow)]
#[case::threshold_one_high(Boundary::ThresholdOneHigh)]
#[case::threshold_just_inside(Boundary::ThresholdJustInside)]
#[case::confidence_at_low(Boundary::ClassificationEdge("0.2", "cloud-fast-1"))]
#[case::confidence_under_low(Boundary::ClassificationEdge("0.1999", "cloud-smart-1"))]
#[case::confidence_at_medium(Boundary::ClassificationEdge("0.5", "local-1"))]
#[case::confidence_under_medium(Boundary::ClassificationEdge("0.4999", "cloud-fast-1"))]
#[case::confidence_at_high(Boundary::ClassificationEdge("0.8", "deep-1"))]
#[case::confidence_under_high(Boundary::ClassificationEdge("0.7999", "local-1"))]
#[case::pick_empty(Boundary::PickEmpty)]
#[case::pick_first_match(Boundary::PickFirstMatch)]
#[case::pick_fallback(Boundary::PickFallback)]
#[case::pick_skips_untyped(Boundary::PickSkipsUntyped)]
#[case::cancelled(Boundary::Cancelled)]
#[case::pending(Boundary::Pending)]
#[tokio::test]
async fn lar1_routing_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::MetadataAtLimit => metadata_at_limit(),
        Boundary::MetadataOverLimit => metadata_over_limit(),
        Boundary::ConfidenceZero => confidence_edge("0", "0"),
        Boundary::ConfidenceOne => confidence_edge("1", "1"),
        Boundary::ConfidenceUnderZero => confidence_out_of_range("-0.001", "-0.001"),
        Boundary::ConfidenceOverOne => confidence_out_of_range("1.001", "1.001"),
        Boundary::MalformedMetadata => malformed_metadata(),
        Boundary::ThresholdOrder => threshold_order(),
        Boundary::ThresholdLowEqualsMedium => threshold_equality("0.5", "0.5", "0.9"),
        Boundary::ThresholdMediumEqualsHigh => threshold_equality("0.3", "0.5", "0.5"),
        Boundary::ThresholdZeroLow => threshold_equality("0", "0.5", "0.7"),
        Boundary::ThresholdOneHigh => threshold_equality("0.3", "0.5", "1"),
        Boundary::ThresholdJustInside => threshold_just_inside(),
        Boundary::ClassificationEdge(confidence, expected) => {
            classification_edge(confidence, expected).await;
        }
        Boundary::PickEmpty => pick_empty(),
        Boundary::PickFirstMatch => pick_first_match(),
        Boundary::PickFallback => pick_fallback(),
        Boundary::PickSkipsUntyped => pick_skips_untyped(),
        Boundary::Cancelled => cancelled().await,
        Boundary::Pending => pending().await,
    }
}

// the fragment at the byte limit still decodes, and one byte more stops before the decoder
fn padded(length: usize) -> Vec<u8> {
    let mut bytes = br#"{"confidence":0.25}"#.to_vec();
    bytes.resize(length, b' ');
    bytes
}

fn metadata_at_limit() {
    let bytes = padded(MAX_METADATA_BYTES);
    assert_eq!(bytes.len(), MAX_METADATA_BYTES);
    let metadata = Lar1Metadata::parse(&bytes).unwrap();
    assert_eq!(metadata.confidence, dec("0.25"));
}

fn metadata_over_limit() {
    let bytes = padded(MAX_METADATA_BYTES + 1);
    let error = Lar1Metadata::parse(&bytes).unwrap_err();
    let MetadataError::TooLarge { length, max } = &error else {
        panic!("an oversized fragment reports its own error");
    };
    assert_eq!(*length, MAX_METADATA_BYTES + 1);
    assert_eq!(*max, MAX_METADATA_BYTES);
    assert_eq!(
        error.to_string(),
        "the lar1 metadata is 1048577 bytes, over the 1048576 byte limit"
    );
}

// the declared confidence bounds are inclusive
fn confidence_edge(confidence: &str, expected: &str) {
    let metadata =
        Lar1Metadata::parse(format!(r#"{{"confidence":{confidence}}}"#).as_bytes()).unwrap();
    assert_eq!(metadata.confidence, dec(expected));
}

fn confidence_out_of_range(confidence: &str, expected: &str) {
    let text = format!(r#"{{"confidence":{confidence}}}"#);
    let error = Lar1Metadata::parse(text.as_bytes()).unwrap_err();
    let MetadataError::Confidence { confidence: got } = &error else {
        panic!("an out-of-range confidence reports its own error");
    };
    assert_eq!(*got, dec(expected));
    assert_eq!(
        error.to_string(),
        format!("the lar1 confidence {expected} is outside the range 0 to 1")
    );
}

fn malformed_metadata() {
    for bytes in [b"{\"act\":".as_slice(), b"[]".as_slice(), b"".as_slice()] {
        let error = Lar1Metadata::parse(bytes).unwrap_err();
        assert!(matches!(error, MetadataError::Invalid { .. }));
        assert!(
            error
                .to_string()
                .starts_with("the lar1 metadata is not a valid LAR-1 object")
        );
    }
    // a JSON array is not a metadata object, and the decode error names the object it expected
    let error = Lar1Metadata::parse(b"[]").unwrap_err();
    assert!(
        error.to_string().contains("a LAR-1 metadata object"),
        "{error}"
    );
}

fn threshold_order() {
    let error =
        Thresholds::from_args(Some(&args(Some("0.7"), Some("0.5"), Some("0.9")))).unwrap_err();
    let ThresholdError::Order { low, medium, high } = &error;
    assert_eq!(*low, dec("0.7"));
    assert_eq!(*medium, dec("0.5"));
    assert_eq!(*high, dec("0.9"));
    assert_eq!(
        error.to_string(),
        "LAR-1 thresholds must satisfy 0 < low < medium < high < 1, got low=0.7, medium=0.5, high=0.9"
    );
}

// every strict inequality is a boundary of its own
fn threshold_equality(low: &str, medium: &str, high: &str) {
    let error =
        Thresholds::from_args(Some(&args(Some(low), Some(medium), Some(high)))).unwrap_err();
    let ThresholdError::Order {
        low: got_low,
        medium: got_medium,
        high: got_high,
    } = &error;
    assert_eq!(*got_low, dec(low));
    assert_eq!(*got_medium, dec(medium));
    assert_eq!(*got_high, dec(high));
}

fn threshold_just_inside() {
    let thresholds =
        Thresholds::from_args(Some(&args(Some("0.001"), Some("0.5"), Some("0.999")))).unwrap();
    assert_eq!(thresholds.low, dec("0.001"));
    assert_eq!(thresholds.medium, dec("0.5"));
    assert_eq!(thresholds.high, dec("0.999"));
}

async fn classification_edge(confidence: &str, expected: &str) {
    let strategy = strategy_with("0.2", "0.5", "0.8");
    let metadata = format!(r#"{{"confidence":{confidence}}}"#);
    let id = selected(
        &strategy,
        Some(&Group(typed_pool())),
        Some(metadata.as_bytes()),
        false,
    )
    .await
    .unwrap();
    assert_eq!(id.as_deref(), Some(expected));
}

fn pick_empty() {
    assert_eq!(pick(&[], Target::Local), None);
}

fn pick_first_match() {
    let pool = vec![
        spec("deep-1", Some("deep")),
        spec("local-1", Some("local")),
        spec("local-2", Some("local")),
    ];
    assert_eq!(pick(&pool, Target::Local), Some((1, true)));
}

fn pick_fallback() {
    let pool = vec![spec("cloud-smart-1", Some("cloud-smart"))];
    assert_eq!(pick(&pool, Target::Local), Some((0, false)));
}

fn pick_skips_untyped() {
    let pool = vec![spec("untyped", None), spec("local-1", Some("local"))];
    assert_eq!(pick(&pool, Target::Local), Some((1, true)));
    let untyped_only = vec![spec("untyped", None)];
    assert_eq!(pick(&untyped_only, Target::Local), Some((0, false)));
}

async fn cancelled() {
    let error = selected(&Lar1Strategy::default(), Some(&Failing), None, false)
        .await
        .unwrap_err();
    assert!(matches!(error, RoutingError::Cancelled));
    assert_eq!(error.unbuilt_entry(), None);
}

async fn pending() {
    // a cancelled caller stops waiting on the healthy pool without inventing a selection
    let strategy = Lar1Strategy::default();
    let router = Pending;
    let selection = selected(&strategy, Some(&router), None, false);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), selection)
            .await
            .is_err()
    );
}
