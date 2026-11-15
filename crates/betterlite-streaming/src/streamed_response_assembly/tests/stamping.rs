use rust_decimal::Decimal;

use crate::streamed_response_assembly::AssemblyError;

use super::support::{failing_logging, priced_logging};
use super::{Harness, chat_of, cost_stream, failed, value};

pub(super) async fn usage_cost_stamping() {
    stamping_calculator_results().await;
    stamping_requires_logging().await;
    stamping_error().await;
}

// The stream's own cost, the logging provider, the calculator's price, the stamped cost, the response cost and the calculator calls.
type StampCase = (
    Option<&'static str>,
    &'static str,
    Decimal,
    Option<Decimal>,
    Option<&'static str>,
    usize,
);

fn stamp_cases() -> [StampCase; 6] {
    let price = Decimal::new(75, 6);
    [
        (None, "openai", price, Some(price), Some("0.000075"), 1),
        (None, "openai", Decimal::ZERO, None, None, 1),
        (None, "xai", price, Some(price), None, 1),
        (
            Some("0"),
            "openai",
            price,
            Some(Decimal::ZERO),
            Some("0"),
            0,
        ),
        (
            Some("-0.5"),
            "openai",
            price,
            Some(Decimal::new(-5, 1)),
            Some("-0.5"),
            0,
        ),
        (
            Some("true"),
            "openai",
            price,
            Some(Decimal::ONE),
            Some("1"),
            0,
        ),
    ]
}

pub(super) async fn stamping_calculator_results() {
    for (reported, provider, price, stamped, response_cost, calls) in stamp_cases() {
        let logging = priced_logging(provider, Some(price));
        let mut harness = Harness::new(Some(cost_stream(reported)));
        harness.logging = Some(&logging);
        let response = chat_of(&harness).await;
        assert_eq!(response.usage.cost, stamped, "{reported:?} {provider}");
        assert_eq!(
            response.hidden_params.get("response_cost"),
            response_cost.map(value).as_ref()
        );
        assert_eq!(logging.calls(), calls);
    }
}

pub(super) async fn stamping_requires_logging() {
    let harness = Harness::new(Some(cost_stream(None)));
    let response = chat_of(&harness).await;
    assert_eq!(response.usage.cost, None);
    assert_eq!(response.hidden_params.get("response_cost"), None);
}

pub(super) async fn stamping_error() {
    let logging = failing_logging("openai");
    let mut harness = Harness::new(Some(cost_stream(None)));
    harness.logging = Some(&logging);
    let (result, logs) = harness.run().await;
    assert!(matches!(failed(result), AssemblyError::Cost { .. }));
    assert!(logs[0].ends_with(
        "Exception occurred - the cost calculation failed: fixture logging cost failure"
    ));
}
