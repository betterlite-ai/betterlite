use super::{Context, State};
use crate::analysis::primitives::{partition_content, partition_items};
use crate::analysis::{AnalysisError, Candidate, Examined, Observation};
use crate::models::execution::{ExecutionContent, TracePart};
use std::collections::BTreeMap;

pub(super) struct Prepared {
    pub supporting: Vec<Observation>,
    pub total_observations: usize,
    pub observation_pages: usize,
    pub catalog: Vec<Examined>,
    pub catalog_pages: usize,
    pub evidence: Vec<TracePart>,
}

pub(super) fn prepare(
    context: &Context<'_>,
    relevant: &[&Examined],
    state: &State,
) -> Result<Prepared, AnalysisError> {
    let observations = observations(context.candidate, relevant);
    let observation_batches = partition_items(&observations, json_size, 16_000)?;
    let supporting = observation_batches
        .get(state.observation_page)
        .cloned()
        .unwrap_or_default();
    let evidence = evidence(
        relevant,
        &state.additional,
        &supporting,
        state.navigation.as_ref(),
    )?;
    let catalog = catalog(context.candidate, context.examined, relevant);
    let catalog_batches = partition_items(&catalog, json_size, 16_000)?;
    let catalog_page = catalog_batches
        .get(state.catalog_page)
        .cloned()
        .unwrap_or_default();
    Ok(Prepared {
        supporting,
        total_observations: observations.len(),
        observation_pages: observation_batches.len(),
        catalog: catalog_page,
        catalog_pages: catalog_batches.len(),
        evidence,
    })
}

fn observations(candidate: &Candidate, relevant: &[&Examined]) -> Vec<Observation> {
    let filtered: Vec<Observation> = relevant
        .iter()
        .flat_map(|item| item.observations.iter())
        .filter(|item| item.check_id == candidate.check_id && item.kind == candidate.kind)
        .cloned()
        .collect();
    debug_assert!(filtered.len() <= relevant.len());
    filtered
}

fn catalog(candidate: &Candidate, examined: &[Examined], relevant: &[&Examined]) -> Vec<Examined> {
    let mut items: Vec<Examined> = relevant.iter().map(|item| (*item).clone()).collect();
    items.extend(
        examined
            .iter()
            .filter(|item| !candidate.execution_ids.contains(&item.execution.id))
            .cloned(),
    );
    items
}

fn json_size<T>(item: &T) -> usize
where
    T: serde::Serialize,
{
    let encoded = serde_json::to_vec(item);
    debug_assert!(encoded.is_ok() || encoded.is_err());
    encoded.map_or(usize::MAX, |value| value.len())
}

fn evidence(
    relevant: &[&Examined],
    additional: &[TracePart],
    supporting: &[Observation],
    navigation: Option<&ExecutionContent>,
) -> Result<Vec<TracePart>, AnalysisError> {
    let cited: Vec<_> = supporting
        .iter()
        .flat_map(|item| item.evidence.iter())
        .map(|item| (item.execution_id.clone(), item.span_id.clone()))
        .collect();
    let mut unique = BTreeMap::new();
    for part in relevant
        .iter()
        .flat_map(|item| item.parts.iter())
        .chain(additional.iter())
    {
        unique.insert(
            (
                part.execution_id.clone(),
                part.span_id.clone(),
                part.content.clone(),
            ),
            part.clone(),
        );
    }
    let recent = navigation.map_or(&[][..], |item| item.parts.as_slice());
    let mut parts: Vec<TracePart> = unique.into_values().collect();
    parts.sort_by_key(|part| {
        (
            !recent.contains(part),
            !cited.contains(&(part.execution_id.clone(), part.span_id.clone())),
            !part.parent_span_id.is_empty(),
            part.kind == "llm",
        )
    });
    Ok(partition_content(&parts, Some(30_000))?
        .into_iter()
        .next()
        .unwrap_or_default())
}
