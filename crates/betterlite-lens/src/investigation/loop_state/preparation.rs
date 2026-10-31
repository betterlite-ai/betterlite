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
    pub all_spans: Vec<TracePart>,
}

pub(super) fn prepare(
    context: &Context<'_>,
    relevant: &[&Examined],
    state: &State,
) -> Result<Prepared, AnalysisError> {
    let observations: Vec<Observation> = relevant
        .iter()
        .flat_map(|item| item.observations.iter())
        .filter(|item| {
            item.check_id == context.candidate.check_id && item.kind == context.candidate.kind
        })
        .cloned()
        .collect();
    let observation_batches = partition_items(&observations, json_size, 16_000)?;
    let supporting = page(&observation_batches, state.observation_page);
    let cited: Vec<(String, String)> = supporting
        .iter()
        .flat_map(|item| item.evidence.iter())
        .map(|item| (item.execution_id.clone(), item.span_id.clone()))
        .collect();
    let all_spans = unique_spans(relevant, &state.additional);
    let evidence = evidence_page(&all_spans, &cited, state.navigation.as_ref())?;
    let catalog_items = catalog(context.candidate, context.examined, relevant);
    let catalog_batches =
        partition_items(&catalog_items, |item| json_size(&item.execution), 16_000)?;
    Ok(Prepared {
        supporting,
        total_observations: observations.len(),
        observation_pages: observation_batches.len(),
        catalog: page(&catalog_batches, state.catalog_page),
        catalog_pages: catalog_batches.len(),
        evidence,
        all_spans,
    })
}

fn json_size<T>(value: &T) -> usize
where
    T: serde::Serialize,
{
    let encoded = serde_json::to_vec(value);
    encoded.map_or(usize::MAX, |bytes| bytes.len())
}

fn page<T>(batches: &[Vec<T>], index: usize) -> Vec<T>
where
    T: Clone,
{
    let selected = batches.get(index).cloned();
    selected.unwrap_or_default()
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

fn unique_spans(relevant: &[&Examined], additional: &[TracePart]) -> Vec<TracePart> {
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
    unique.into_values().collect()
}

fn evidence_page(
    all_spans: &[TracePart],
    cited: &[(String, String)],
    navigation: Option<&ExecutionContent>,
) -> Result<Vec<TracePart>, AnalysisError> {
    let recent = navigation.map_or(&[][..], |item| item.parts.as_slice());
    let mut parts = all_spans.to_vec();
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
