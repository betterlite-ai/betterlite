#![cfg(test)]

use super::shared::airline_checker;

#[test]
fn no_comparison_language_yields_055_confidence() {
    assert_eq!(
        airline_checker()
            .run("Qatar airline flights available now")
            .confidence,
        Some(0.55_f64)
    );
}
