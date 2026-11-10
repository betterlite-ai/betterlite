#![cfg(test)]

use super::DEFAULT_TIMEOUT_SECONDS;

#[test]
fn the_default_is_six_hundred_seconds() {
    assert_eq!(DEFAULT_TIMEOUT_SECONDS.to_bits(), 600.0_f64.to_bits());
}
