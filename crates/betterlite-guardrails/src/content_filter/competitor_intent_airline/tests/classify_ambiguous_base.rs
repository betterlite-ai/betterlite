#![cfg(test)]

use crate::content_filter::competitor_intent_base::{Meaning, normalize};

use super::shared::base_config;
use crate::content_filter::competitor_intent_base::BaseCompetitorIntentChecker;

#[test]
fn ambiguous_competitor_location_token_is_treated_as_non_competitor() {
    let checker = BaseCompetitorIntentChecker::new(&base_config()).unwrap();
    let result = checker.evaluate(&normalize("qatar"), |_, _| (Meaning::OtherMeaning, 0.5_f64));
    assert_eq!(result.intent.as_deref(), Some("other"));
}
