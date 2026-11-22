use std::sync::PoisonError;

use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_router::simple_shuffle::error::ShuffleError;
use betterlite_router::simple_shuffle::{Candidate, pick};

pub fn pick_deployment<'executor>(
    executor: &'executor ChatExecutor,
    model: &str,
) -> Result<&'executor ChatDeployment, ChatError> {
    let group: Vec<&ChatDeployment> = executor
        .deployments
        .iter()
        .filter(|deployment| deployment.model_group == model)
        .collect();
    pick_matching(executor, model, &group, "a model group")
}

// Router picks a model-group deployment and a litellm-model deployment with the same weighted shuffle
pub fn pick_matching<'executor>(
    executor: &'executor ChatExecutor,
    model: &str,
    matching: &[&'executor ChatDeployment],
    subject: &str,
) -> Result<&'executor ChatDeployment, ChatError> {
    let candidates: Vec<Candidate> = matching
        .iter()
        .map(|deployment| Candidate {
            weight: deployment.weight,
            rpm: deployment.rpm,
            tpm: deployment.tpm,
        })
        .collect();
    let unknown = || ChatError::UnknownModel {
        model: model.to_owned(),
    };
    let picked = {
        let mut random = executor
            .random
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        pick(&candidates, &mut *random)
    };
    let index = match picked {
        Ok(index) => index,
        Err(ShuffleError::NoCandidates) => return Err(unknown()),
        Err(error @ ShuffleError::Weights { .. }) => {
            tracing::error!(%error, model, "the deployments of {subject} could not be weighed");
            return Err(unknown());
        }
    };
    matching.get(index).copied().ok_or_else(unknown)
}

mod tests;
