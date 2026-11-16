use betterlite_config::sections::guardrail_params::records::LakeraCategoryThresholds;

use super::rejection::LakeraViolation;
use super::response::CategoryScores;

// with thresholds configured, a score blocks only when its own threshold is set
pub(super) fn violation_of(
    scores: &CategoryScores,
    thresholds: LakeraCategoryThresholds,
) -> Option<LakeraViolation> {
    if exceeds(scores.jailbreak.flatten(), thresholds.jailbreak) {
        return Some(LakeraViolation::Jailbreak);
    }
    if exceeds(
        scores.prompt_injection.flatten(),
        thresholds.prompt_injection,
    ) {
        return Some(LakeraViolation::PromptInjection);
    }
    None
}

fn exceeds(score: Option<f64>, threshold: Option<f64>) -> bool {
    match (score, threshold) {
        (Some(measured), Some(limit)) => measured >= limit,
        _ => false,
    }
}
