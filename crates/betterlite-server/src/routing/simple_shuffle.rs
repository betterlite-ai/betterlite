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
    let candidates: Vec<Candidate> = group
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
            tracing::error!(%error, model, "the deployments of a model group could not be weighed");
            return Err(unknown());
        }
    };
    group.get(index).copied().ok_or_else(unknown)
}

mod tests;
