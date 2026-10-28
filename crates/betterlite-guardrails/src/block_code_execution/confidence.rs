use super::language_normalization::{ConfidenceMode, confidence_for_block};

#[test]
fn scores_non_executable_lower() {
    assert_eq!(
        confidence_for_block("text", ConfidenceMode::All).to_bits(),
        0.5_f64.to_bits()
    );
}
