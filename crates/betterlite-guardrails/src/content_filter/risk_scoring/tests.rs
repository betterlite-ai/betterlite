#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::content_filter_records::{
    ContentFilterDetection, PatternDetection,
};

use super::{GuardrailStatus, compute_risk_score};

fn detection() -> ContentFilterDetection {
    ContentFilterDetection::Pattern(PatternDetection {
        r#type: "pattern".to_owned(),
        pattern_name: "email".to_owned(),
        action: "MASK".to_owned(),
    })
}

#[test]
fn intervention_is_maximum() {
    assert_eq!(
        compute_risk_score(
            &[],
            &BTreeMap::new(),
            0,
            GuardrailStatus::GuardrailIntervened
        )
        .to_bits(),
        10.0_f64.to_bits()
    );
}

#[test]
fn score_has_mask_floor_and_rounding() {
    let mut masked = BTreeMap::new();
    masked.insert("email".to_owned(), 1);
    assert_eq!(
        compute_risk_score(&[detection()], &masked, 10, GuardrailStatus::Success).to_bits(),
        2.0_f64.to_bits()
    );
    assert_eq!(
        compute_risk_score(
            &[detection(), detection()],
            &BTreeMap::new(),
            0,
            GuardrailStatus::Success
        )
        .to_bits(),
        1.2_f64.to_bits()
    );
}
