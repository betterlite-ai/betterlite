#![cfg(test)]

use super::shared::airline_checker;

#[test]
fn comparison_language_yields_075_confidence() {
    assert_eq!(
        airline_checker().run("Qatar Airways vs us").confidence,
        Some(0.75_f64)
    );
}
