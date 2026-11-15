#![cfg(test)]

use betterlite_logging::logging_privacy::CallbackMetadataError;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rust_decimal::Decimal;

use super::{PositionCoin, UniformCoin, metadata_failure};

#[test]
fn metadata_failure_names_the_callback_error_and_costs_nothing() {
    let failure = metadata_failure(CallbackMetadataError::InvalidField {
        field: "metadata".to_owned(),
    });

    assert_eq!(
        failure.error,
        "CallbackMetadataError: invalid callback field metadata"
    );
    assert_eq!(failure.cost, Decimal::ZERO);
    assert_eq!(failure.classifier_cost, Decimal::ZERO);
}

#[test]
fn the_uniform_coin_draws_both_positions() {
    let coin = UniformCoin::new(StdRng::seed_from_u64(7));

    let draws: Vec<bool> = (0..64).map(|_| coin.real_is_a()).collect();

    assert!(draws.iter().any(|value| *value));
    assert!(draws.iter().any(|value| !*value));
}
