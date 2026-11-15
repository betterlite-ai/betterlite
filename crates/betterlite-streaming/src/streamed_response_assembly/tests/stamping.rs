use rust_decimal::Decimal;

use crate::streamed_response_assembly::AssemblyError;

use super::support::FixtureLogging;
use super::{Harness, chat_of, cost_stream, failed, value};

pub(super) async fn usage_cost_stamping() {
    stamping_from_logging().await;
    stamping_requires_logging().await;
    stamping_skips_existing().await;
    stamping_zero_calculator().await;
    stamping_still_applies_to_xai().await;
    stamping_error().await;
}

pub(super) async fn stamping_from_logging() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("openai");
    logging.cost = Some(Decimal::new(75, 6));
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::new(75, 6)));
    assert_eq!(
        response.hidden_params.get("response_cost"),
        Some(&value("0.000075"))
    );
    assert_eq!(logging.calls(), 1);
}

pub(super) async fn stamping_requires_logging() {
    let harness = Harness::new(Some(cost_stream(None)));
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, None);
    assert_eq!(response.hidden_params.get("response_cost"), None);
}

pub(super) async fn stamping_skips_existing() {
    let existing = [
        ("0", Decimal::ZERO),
        ("-0.5", Decimal::new(-5, 1)),
        ("true", Decimal::ONE),
    ];
    for (reported, expected) in existing {
        let mut logging = FixtureLogging::default();
        logging.provider = Some("openai");
        logging.cost = Some(Decimal::new(75, 6));
        let mut harness = Harness::new(Some(cost_stream(Some(reported))));
        harness.logging = Some(&logging);
        let response = chat_of(&harness).await;
        assert_eq!(response.usage.cost, Some(expected));
        assert_eq!(
            response.hidden_params.get("response_cost"),
            Some(&value(&expected.to_string()))
        );
        assert_eq!(logging.calls(), 0);
    }
}

pub(super) async fn stamping_zero_calculator() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("openai");
    logging.cost = Some(Decimal::ZERO);
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, None);
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(logging.calls(), 1);
}

pub(super) async fn stamping_still_applies_to_xai() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("xai");
    logging.cost = Some(Decimal::new(75, 6));
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, Some(Decimal::new(75, 6)));
    assert_eq!(response.hidden_params.get("response_cost"), None);
    assert_eq!(logging.calls(), 1);
}

pub(super) async fn stamping_error() {
    let mut logging = FixtureLogging::default();
    logging.provider = Some("openai");
    logging.fail = true;
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    let (result, logs) = harness.run().await;
    assert!(matches!(failed(result), AssemblyError::Cost(_)));
    assert!(logs[0].ends_with(
        "Exception occurred - the cost calculation failed: fixture logging cost failure"
    ));
}
