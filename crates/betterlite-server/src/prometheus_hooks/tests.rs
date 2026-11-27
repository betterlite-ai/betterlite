mod support;

use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use rust_decimal::Decimal;

use self::support::{FAILED, REQUESTS, dec, event, fixture, payload, totals};

#[tokio::test]
async fn callback_logger_runs_both_events() {
    let fixture = fixture();
    assert_eq!(fixture.hooks.callback_name(), "PrometheusLogger");
    let success = event(payload(Decimal::ZERO));
    fixture
        .hooks
        .async_log_success_event(Arc::new(success))
        .await
        .unwrap();
    let failure = event(payload(dec("0.5")));
    fixture
        .hooks
        .async_log_failure_event(Arc::new(failure))
        .await
        .unwrap();
    assert_eq!(totals(&fixture.registry, REQUESTS), vec![Decimal::ONE]);
    assert_eq!(totals(&fixture.registry, FAILED), vec![Decimal::ONE]);
}
