use super::language_normalization::{ConfidenceMode, confidence_for_block, is_blocked_language};

#[test]
fn aliases_and_confidence_are_normalized() {
    assert!(is_blocked_language(
        "JS",
        Some(&["javascript".to_owned()]),
        false
    ));
    assert_eq!(
        confidence_for_block("text", ConfidenceMode::All).to_bits(),
        0.5_f64.to_bits()
    );
}
