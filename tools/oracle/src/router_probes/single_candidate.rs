use betterlite_router::simple_shuffle::{Candidate, pick};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub deployments: Vec<Deployment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    pub id: String,
    #[serde(default)]
    pub weight: u32,
    #[serde(default)]
    pub rpm: u32,
    #[serde(default)]
    pub tpm: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub selected: String,
}

pub(crate) fn run(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let candidates: Vec<Candidate> = input
        .deployments
        .iter()
        .map(|deployment| Candidate {
            weight: deployment.weight,
            rpm: deployment.rpm,
            tpm: deployment.tpm,
        })
        .collect();
    let index = pick(&candidates, random).map_err(|error| OperationError::InvalidRequest {
        message: error.to_string(),
    })?;
    // LiteLLM makes one draw for this selection, so a rejected sample was still a draw
    if random.next == 0 {
        return Err(OperationError::SamplesExhausted {
            message: "the request has no sample left for this draw".to_owned(),
        });
    }
    let selected = input
        .deployments
        .get(index)
        .ok_or_else(|| OperationError::InvalidRequest {
            message: "the selected deployment is out of range".to_owned(),
        })?;
    Ok(ProbeOutput::SingleCandidate(Output {
        selected: selected.id.clone(),
    }))
}
