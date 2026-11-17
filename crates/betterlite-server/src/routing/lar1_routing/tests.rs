#![cfg(test)]

use betterlite_config::sections::lar1_routing as config;
use betterlite_router::lar1_routing::thresholds::{ThresholdError, Thresholds};
use rust_decimal::Decimal;

use super::resolve;

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn configured(low: Option<&str>, medium: Option<&str>, high: Option<&str>) -> config::Settings {
    config::Settings {
        args: Some(config::Args {
            low: low.map(dec),
            medium: medium.map(dec),
            high: high.map(dec),
        }),
    }
}

#[test]
fn lar1_routing_resolve_keeps_the_declared_defaults() {
    let strategy = resolve(&config::Settings::default()).unwrap();
    assert_eq!(strategy.thresholds, Thresholds::default());
    assert_eq!(strategy.thresholds.low, dec("0.3"));
    assert_eq!(strategy.thresholds.medium, dec("0.5"));
    assert_eq!(strategy.thresholds.high, dec("0.7"));
}

#[test]
fn lar1_routing_resolve_uses_the_configured_thresholds() {
    let strategy = resolve(&configured(Some("0.2"), Some("0.5"), Some("0.8"))).unwrap();
    assert_eq!(strategy.thresholds.low, dec("0.2"));
    assert_eq!(strategy.thresholds.medium, dec("0.5"));
    assert_eq!(strategy.thresholds.high, dec("0.8"));
}

#[test]
fn lar1_routing_resolve_keeps_the_default_for_an_absent_threshold() {
    let strategy = resolve(&configured(Some("0.2"), None, None)).unwrap();
    assert_eq!(strategy.thresholds.low, dec("0.2"));
    assert_eq!(strategy.thresholds.medium, dec("0.5"));
    assert_eq!(strategy.thresholds.high, dec("0.7"));
}

#[test]
fn lar1_routing_resolve_rejects_an_unordered_configuration() {
    let error = resolve(&configured(Some("0.7"), Some("0.5"), Some("0.9"))).unwrap_err();
    assert!(matches!(error, ThresholdError::Order { .. }));
    assert_eq!(
        error.to_string(),
        "LAR-1 thresholds must satisfy 0 < low < medium < high < 1, got low=0.7, medium=0.5, high=0.9"
    );
}
