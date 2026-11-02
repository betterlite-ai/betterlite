use super::prompt::cluster_request;
use crate::analysis::primitives::structured_response;
use crate::analysis::{AnalysisError, Candidate, Clusters, ModelCall};
use crate::models::messages::{ModelRequest, Purpose};
use indexmap::IndexMap;
use std::collections::HashSet;

struct Identity {
    index: usize,
    candidate: Candidate,
}

pub(super) async fn merge_candidates(
    candidates: Vec<Candidate>,
    prior_count: usize,
    model: &ModelCall,
) -> Result<(Vec<Candidate>, Vec<Candidate>), AnalysisError> {
    let identities: IndexMap<String, Identity> = candidates
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| (format!("p{index}"), Identity { index, candidate }))
        .collect();
    let aliased: Vec<Candidate> = identities
        .iter()
        .map(|(identity, slot)| Candidate {
            execution_ids: vec![identity.clone()],
            ..slot.candidate.clone()
        })
        .collect();
    let request = ModelRequest {
        prompt: cluster_request(aliased)?,
        purpose: Purpose::Cluster,
    };
    let response: Clusters =
        structured_response(request, |sent| model(sent), validate_groups).await?;
    let valid: Vec<Candidate> = response
        .candidates
        .into_iter()
        .filter(|candidate| {
            !candidate.execution_ids.is_empty()
                && candidate.execution_ids.iter().all(|identity| {
                    identities.get(identity).is_some_and(|slot| {
                        slot.candidate.check_id == candidate.check_id
                            && slot.candidate.kind == candidate.kind
                    })
                })
        })
        .collect();
    Ok(split_active(expand_all(valid, &identities, prior_count)))
}

fn validate_groups(groups: &Clusters) -> Option<String> {
    let mut seen = HashSet::new();
    for candidate in &groups.candidates {
        for identity in &candidate.execution_ids {
            if !seen.insert(identity.clone()) {
                return Some(String::from(
                    "Each input reference must appear in exactly one group; do not duplicate it across findings.",
                ));
            }
        }
    }
    None
}

fn expand_all(
    valid: Vec<Candidate>,
    identities: &IndexMap<String, Identity>,
    prior_count: usize,
) -> Vec<(Candidate, bool)> {
    let used: HashSet<String> = valid
        .iter()
        .flat_map(|candidate| candidate.execution_ids.iter().cloned())
        .collect();
    let mut preserved: Vec<(Candidate, bool)> = valid
        .into_iter()
        .map(|candidate| expand_candidate(candidate, identities, prior_count))
        .collect();
    for (identity, slot) in identities {
        if !used.contains(identity) {
            preserved.push((slot.candidate.clone(), slot.index >= prior_count));
        }
    }
    preserved
}

fn expand_candidate(
    candidate: Candidate,
    identities: &IndexMap<String, Identity>,
    prior_count: usize,
) -> (Candidate, bool) {
    let mut execution_ids: Vec<String> = candidate
        .execution_ids
        .iter()
        .filter_map(|identity| identities.get(identity))
        .flat_map(|slot| slot.candidate.execution_ids.iter().cloned())
        .collect();
    execution_ids.sort();
    execution_ids.dedup();
    let active = candidate.execution_ids.iter().any(|identity| {
        identities
            .get(identity)
            .is_some_and(|slot| slot.index >= prior_count)
    });
    (
        Candidate {
            execution_ids,
            ..candidate
        },
        active,
    )
}

fn split_active(preserved: Vec<(Candidate, bool)>) -> (Vec<Candidate>, Vec<Candidate>) {
    let mut active = Vec::new();
    let mut settled = Vec::new();
    for (candidate, is_active) in preserved {
        if is_active {
            active.push(candidate);
        } else {
            settled.push(candidate);
        }
    }
    (active, settled)
}
