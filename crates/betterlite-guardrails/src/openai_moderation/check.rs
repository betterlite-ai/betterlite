use super::error::ModerationError;
use super::wire::{ModerationResponse, ViolationDetails};

// LiteLLM blocks on the first result only, so later flagged results never raise.
pub(super) fn check_moderation_result(
    moderation_response: &ModerationResponse,
) -> Result<(), ModerationError> {
    let Some(result) = moderation_response.results.first() else {
        return Ok(());
    };
    if result.flagged != Some(true) {
        return Ok(());
    }
    let violated_categories = result
        .categories
        .iter()
        .flatten()
        .filter(|(_, is_violated)| **is_violated)
        .map(|(category, _)| category.clone())
        .collect();
    let details = ViolationDetails {
        violated_categories,
        category_scores: result.category_scores.clone().unwrap_or_default(),
    };
    tracing::warn!(
        violated_categories = ?details.violated_categories,
        "OpenAI Moderation: content flagged for violations"
    );
    Err(ModerationError::Flagged { details })
}
