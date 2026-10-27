use super::storage::Summary;
use crate::analysis::primitives::evidence_valid;
use crate::analysis::{AnalysisError, Examined, Observation, TraceReview};
use crate::models::execution::{Execution, ExecutionContent, TracePart};
use crate::models::settings::Check;
use crate::trace_store::TraceStore;
use std::collections::HashSet;

pub(super) struct Verified {
    observations: Vec<Observation>,
    parts: Vec<TracePart>,
    partial: bool,
    cannot_assess: bool,
}

pub(super) fn verify(
    response: TraceReview,
    store: &TraceStore,
    checks: &[Check],
) -> Result<Verified, AnalysisError> {
    let mut parts = Vec::new();
    for evidence in response.observations.iter().flat_map(|item| &item.evidence) {
        if let Some(part) = store.evidence(evidence)?
            && !parts.contains(&part)
        {
            parts.push(part);
        }
    }
    let original_count = response.observations.len();
    let observations: Vec<_> = response
        .observations
        .into_iter()
        .filter(|item| {
            checks.iter().any(|check| check.id == item.check_id)
                && !item.evidence.is_empty()
                && item
                    .evidence
                    .iter()
                    .all(|evidence| evidence_valid(evidence, &parts))
        })
        .collect();
    let partial = observations.len() != original_count || !response.reads.is_empty();
    Ok(Verified {
        observations,
        parts,
        partial,
        cannot_assess: partial || response.cannot_assess,
    })
}

pub(super) fn combine(
    execution: &Execution,
    reviews: &[Verified],
    summary: &Summary,
    page: &ExecutionContent,
) -> Examined {
    let observations: Vec<_> = reviews
        .iter()
        .flat_map(|review| review.observations.clone())
        .collect();
    let cited: HashSet<_> = observations
        .iter()
        .flat_map(|item| item.evidence.iter().map(|evidence| &evidence.span_id))
        .collect();
    let mut parts = Vec::new();
    for part in reviews
        .iter()
        .flat_map(|review| &review.parts)
        .chain(summary.root.iter())
    {
        if (cited.contains(&part.span_id) || part.parent_span_id.is_empty())
            && !parts.contains(part)
        {
            parts.push(part.clone());
        }
    }
    Examined {
        execution: execution.clone(),
        observations,
        parts,
        partial: reviews
            .iter()
            .any(|review| review.partial || page.partial || page.next_cursor.is_some()),
        cannot_assess: reviews.is_empty()
            || summary.span_count == 0
            || reviews.iter().all(|review| review.cannot_assess),
    }
}
