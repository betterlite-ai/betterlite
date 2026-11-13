use betterlite_models::model_capability_predicates::CapabilityPredicates;
use betterlite_roi::estimator::evidence::metadata_evidence;
use betterlite_roi::estimator::extraction::first_json_object;
use betterlite_roi::estimator::json::json_dumps_text;
use betterlite_roi::estimator::models::{
    MAX_EVIDENCE_CHARS, REASONING_MAX_CHARS, RESPONSE_CONTRACT, ROICompletionMessage,
    ROICompletionMetadata, ROICompletionRequest, ROICompletionResponse, ROICompletionRole,
    ROIResponseFormat, ROIResponseFormatType, ReasoningEffort,
};
use betterlite_roi::estimator::options::estimator_options;
use betterlite_roi::estimator::result::ROIEstimatorResult;
use betterlite_roi::records::{ROIEstimate, ROIEstimateStatus, ROIPullEvidence};
use serde_json::Value;

use super::Estimator;
use super::error::{CompletionFailure, EstimatorError};

pub const INCOMPLETE_METADATA_REASON: &str =
    "GitHub did not provide all file or commit metadata. It was not sent for estimation.";
pub const OVERSIZED_EVIDENCE_REASON: &str =
    "This PR exceeds the estimator's input limit. It was not truncated or scored.";

impl Estimator {
    pub async fn estimate<Predicates>(
        &self,
        pull: &ROIPullEvidence,
        predicates: &mut Predicates,
    ) -> Result<ROIEstimate, EstimatorError>
    where
        Predicates: CapabilityPredicates + Send,
    {
        let evidence =
            json_dumps_text(&metadata_evidence(pull)).map_err(|source| EstimatorError {
                source: CompletionFailure::Evidence { source },
            })?;
        if pull.incomplete_metadata {
            return Ok(ROIEstimate {
                status: ROIEstimateStatus::NeedsReview,
                hours: None,
                reasoning: INCOMPLETE_METADATA_REASON.to_owned(),
                model: None,
                evidence_source: None,
                effort_basis: None,
                cached: None,
            });
        }
        if evidence.chars().count() > MAX_EVIDENCE_CHARS {
            return Ok(ROIEstimate {
                status: ROIEstimateStatus::NeedsReview,
                hours: None,
                reasoning: OVERSIZED_EVIDENCE_REASON.to_owned(),
                model: None,
                evidence_source: None,
                effort_basis: None,
                cached: None,
            });
        }
        let options = estimator_options(predicates, &self.models).await;
        let request = self.completion_request(evidence, options.reasoning_effort);
        let value = (self.complete)(request)
            .await
            .map_err(|source| EstimatorError {
                source: CompletionFailure::Callback { source },
            })?;
        estimate_from_completion(value, &self.settings.estimator_model)
    }

    fn completion_request(
        &self,
        evidence: String,
        reasoning_effort: Option<ReasoningEffort>,
    ) -> ROICompletionRequest {
        ROICompletionRequest {
            model: self.settings.estimator_model.clone(),
            temperature: 0,
            messages: vec![
                ROICompletionMessage {
                    role: ROICompletionRole::System,
                    content: format!("{}\n\n{RESPONSE_CONTRACT}", self.settings.estimator_prompt),
                },
                ROICompletionMessage {
                    role: ROICompletionRole::User,
                    content: evidence,
                },
            ],
            response_format: ROIResponseFormat {
                r#type: ROIResponseFormatType::JsonObject,
            },
            max_tokens: 1200,
            metadata: ROICompletionMetadata {
                tags: vec!["litellm-roi-estimator".to_owned()],
                litellm_roi_estimator: true,
            },
            reasoning_effort,
        }
    }
}

fn estimate_from_completion(value: Value, model: &str) -> Result<ROIEstimate, EstimatorError> {
    let response: ROICompletionResponse =
        serde_json::from_value(value).map_err(|source| EstimatorError {
            source: CompletionFailure::Response { source },
        })?;
    let choice = response.choices.first().ok_or_else(|| EstimatorError {
        source: CompletionFailure::NoChoices,
    })?;
    if !matches!(choice.finish_reason.as_deref(), None | Some("stop")) {
        return Err(EstimatorError {
            source: CompletionFailure::Incomplete,
        });
    }
    let content = choice
        .message
        .content
        .as_deref()
        .ok_or_else(|| EstimatorError {
            source: CompletionFailure::Incomplete,
        })?;
    let result: ROIEstimatorResult =
        serde_json::from_str(first_json_object(content)).map_err(|source| EstimatorError {
            source: CompletionFailure::Estimate { source },
        })?;
    Ok(ROIEstimate {
        status: ROIEstimateStatus::Estimated,
        hours: Some(result.hours),
        reasoning: result.reasoning.chars().take(REASONING_MAX_CHARS).collect(),
        model: Some(model.to_owned()),
        evidence_source: Some("pr_metadata".to_owned()),
        effort_basis: Some("without_ai".to_owned()),
        cached: Some(false),
    })
}
