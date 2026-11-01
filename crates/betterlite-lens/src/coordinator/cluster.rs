use super::batches::kind_order;
use super::merge::merge_candidates;
use crate::analysis::primitives::partition_items;
use crate::analysis::{
    AnalysisError, Candidate, Clusters, ModelCall, Observation, ObservationKind, ReportProgress,
};
use crate::models::execution::Coverage;
use std::collections::HashSet;

pub(super) const fn candidate_size(candidate: &Candidate) -> usize {
    const JSON_OVERHEAD_BYTES: usize = 200;
    candidate
        .title
        .len()
        .saturating_add(candidate.hypothesis.len())
        .saturating_add(candidate.check_id.len())
        .saturating_add(JSON_OVERHEAD_BYTES)
}

pub(super) async fn cluster_batches(
    batches: Vec<Vec<Observation>>,
    model: ModelCall,
    progress: ReportProgress,
    coverage: Coverage,
) -> Result<Clusters, AnalysisError> {
    let candidates = accumulate(batches, &model, &progress, &coverage).await?;
    let mut ordered = candidates;
    ordered.sort_by_key(|candidate| (candidate.check_id.clone(), kind_order(candidate.kind)));
    let registry = settle(ordered, &model).await?;
    Ok(Clusters {
        candidates: registry,
    })
}

async fn accumulate(
    batches: Vec<Vec<Observation>>,
    model: &ModelCall,
    progress: &ReportProgress,
    coverage: &Coverage,
) -> Result<Vec<Candidate>, AnalysisError> {
    let mut candidates: Vec<Candidate> = Vec::new();
    for (index, batch) in batches.into_iter().enumerate() {
        let mut current = coverage.clone();
        current.grouped_batches =
            i64::try_from(index).map_err(|error| AnalysisError::Validation(error.to_string()))?;
        progress(String::from("Grouping observations"), current).await?;
        candidates = consolidate(batch, candidates, model).await?;
    }
    Ok(candidates)
}

pub(super) async fn consolidate(
    batch: Vec<Observation>,
    previous: Vec<Candidate>,
    model: &ModelCall,
) -> Result<Vec<Candidate>, AnalysisError> {
    let drafted: Vec<Candidate> = batch.iter().map(observation_to_candidate).collect();
    paginate_merge(previous, drafted, model).await
}

async fn paginate_merge(
    previous: Vec<Candidate>,
    mut active: Vec<Candidate>,
    model: &ModelCall,
) -> Result<Vec<Candidate>, AnalysisError> {
    let mut retained: Vec<Candidate> = Vec::new();
    for prior in pages_or_single(partition_items(&previous, candidate_size, 16000)?) {
        let prior_count = prior.len();
        let combined: Vec<Candidate> = prior.into_iter().chain(active).collect();
        let (continued, settled) = merge_candidates(combined, prior_count, model).await?;
        active = continued;
        retained.extend(settled);
    }
    retained.extend(active);
    Ok(retained)
}

pub(super) async fn settle(
    candidates: Vec<Candidate>,
    model: &ModelCall,
) -> Result<Vec<Candidate>, AnalysisError> {
    let mut registry: Vec<Candidate> = Vec::new();
    for incoming in partition_items(&candidates, candidate_size, 8000)? {
        registry = settle_page(registry, incoming, model).await?;
    }
    Ok(registry)
}

pub(super) async fn settle_page(
    registry: Vec<Candidate>,
    incoming: Vec<Candidate>,
    model: &ModelCall,
) -> Result<Vec<Candidate>, AnalysisError> {
    let keys: HashSet<(String, u8)> = incoming
        .iter()
        .map(|candidate| (candidate.check_id.clone(), kind_order(candidate.kind)))
        .collect();
    let (matching, unrelated): (Vec<Candidate>, Vec<Candidate>) =
        registry.into_iter().partition(|candidate| {
            keys.contains(&(candidate.check_id.clone(), kind_order(candidate.kind)))
        });
    let merged = paginate_merge(matching, incoming, model).await?;
    Ok(unrelated.into_iter().chain(merged).collect())
}

fn pages_or_single(pages: Vec<Vec<Candidate>>) -> Vec<Vec<Candidate>> {
    if pages.is_empty() {
        vec![Vec::new()]
    } else {
        pages
    }
}

fn observation_to_candidate(observation: &Observation) -> Candidate {
    let mut execution_ids: Vec<String> = observation
        .evidence
        .iter()
        .map(|evidence| evidence.execution_id.clone())
        .collect();
    execution_ids.sort();
    execution_ids.dedup();
    Candidate {
        check_id: observation.check_id.clone(),
        kind: observation.kind,
        title: observation.summary.chars().take(160).collect(),
        hypothesis: format!("{}: {}", kind_label(observation.kind), observation.summary),
        execution_ids,
        existing_finding_id: None,
    }
}

const fn kind_label(kind: ObservationKind) -> &'static str {
    match kind {
        ObservationKind::Issue => "issue",
        ObservationKind::Pattern => "pattern",
    }
}
