#![cfg(test)]

use super::Meaning;

#[test]
fn classify_ambiguous() {
    assert_eq!(
        (Meaning::OtherMeaning, 0.5_f64),
        (Meaning::OtherMeaning, 0.5_f64)
    );
}
