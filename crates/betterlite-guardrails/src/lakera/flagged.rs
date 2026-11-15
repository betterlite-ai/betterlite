use betterlite_config::sections::guardrail_params::records::LakeraCategoryThresholds;

use super::LegacyLakeraModeration;
use super::rejection::{LakeraRejection, LakeraViolation};
use super::response::{CategoryScores, LakeraModerationResponse};

impl LegacyLakeraModeration {
    // LiteLLM's _check_response_flagged: only results[0] decides, and a score blocks only when its own threshold is set
    #[must_use]
    pub fn check_response_flagged(
        &self,
        response: &LakeraModerationResponse,
    ) -> Option<LakeraRejection> {
        let result = response.results.as_ref()?.as_ref()?.first()?;
        let Some(thresholds) = self.category_thresholds else {
            return matches!(result.flagged, Some(Some(true)))
                .then(|| LakeraRejection::new(LakeraViolation::ContentSafetyPolicy, response));
        };
        // with thresholds configured, a response that carries no scores passes even when it is flagged
        let scores = result.category_scores.as_ref()?.as_ref()?;
        violation_of(scores, thresholds).map(|violation| LakeraRejection::new(violation, response))
    }
}

fn violation_of(
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
        (Some(score), Some(threshold)) => score >= threshold,
        _ => false,
    }
}
