#![cfg(test)]

use super::shared::airline_checker;

#[test]
fn category_ranking_yields_065_confidence() {
    assert_eq!(
        airline_checker()
            .run("which carrier is ranked best")
            .confidence,
        Some(0.65_f64)
    );
}
