pub mod coordinator;
mod loop_state;
mod prompt;
mod validation;

use crate::analysis::primitives::structured_response;
use crate::analysis::{
    AnalysisError, Candidate, Decision, DecisionAction, Examined, FinalAction, FinalDecision,
    Investigation, ModelCall, ReadContent,
};
use crate::models::messages::{Claim, ModelRequest};

const PROMPT_LIMIT: usize = 100_000;

pub async fn investigate(
    claim: Claim,
    candidate: Candidate,
    examined: Vec<Examined>,
    read: ReadContent,
    model: ModelCall,
) -> Result<Investigation, AnalysisError> {
    let context = loop_state::Context {
        claim: &claim,
        candidate: &candidate,
        examined: &examined,
        read: &read,
        model: &model,
    };
    match loop_state::run(context).await {
        Ok(result) => Ok(result),
        Err(AnalysisError::Validation(_) | AnalysisError::Decode(_)) => Ok(Investigation {
            finding: None,
            parts: Vec::new(),
        }),
        Err(error) => Err(error),
    }
}

pub async fn investigation_decision(
    request: ModelRequest,
    model: &ModelCall,
    steps: usize,
) -> Result<Decision, AnalysisError> {
    if steps > 1 {
        return structured_response(request, |sent| model(sent), |_| None).await;
    }
    let final_decision: FinalDecision =
        structured_response(request, |sent| model(sent), |_| None).await?;
    Ok(Decision {
        action: match final_decision.action {
            FinalAction::Submit => DecisionAction::Submit,
            FinalAction::Inconclusive => DecisionAction::Inconclusive,
        },
        page: 0,
        execution_id: None,
        cursor: String::new(),
        offset: 0,
        finding: final_decision.finding,
    })
}

#[path = "investigation/tests/decision_navigation.rs"]
mod decision_navigation;
#[path = "investigation/tests/decision_schema.rs"]
mod decision_schema;
#[path = "investigation/tests/evidence_validation.rs"]
mod evidence_validation;
#[path = "investigation/tests/fixtures.rs"]
mod fixtures;
#[path = "investigation/tests/investigate.rs"]
mod investigate;
#[path = "investigation/tests/investigate_candidates.rs"]
mod investigate_candidates;
#[path = "investigation/tests/investigate_stored.rs"]
mod investigate_stored;
#[path = "investigation/tests/prompt_size_exit.rs"]
mod prompt_size_exit;
#[path = "investigation/tests/repeated_request.rs"]
mod repeated_request;
