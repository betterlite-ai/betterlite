#![cfg(test)]

use super::BaseCompetitorIntentChecker;
use super::Meaning;

#[test]
fn classify_ambiguous() {
    assert_eq!(
        BaseCompetitorIntentChecker::classify_ambiguous("acme", "acme"),
        (Meaning::OtherMeaning, 0.5_f64)
    );
}
