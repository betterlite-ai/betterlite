use super::{PROMPT_LIMIT, investigation_decision};
use super::{prompt, validation};
use crate::analysis::primitives::{
    FeedbackRow, feedback_pages, partition_content, partition_items,
};
use crate::analysis::{
    AnalysisError, Candidate, Decision, DecisionAction, Examined, Investigation, ModelCall,
    Observation, ReadContent,
};
use crate::models::execution::{ExecutionContent, TracePart};
use crate::models::messages::{Claim, ModelRequest, Purpose};
use crate::trace_store::trace_store;
use std::collections::BTreeMap;
mod navigation;

pub(super) struct Context<'context> {
    pub claim: &'context Claim,
    pub candidate: &'context Candidate,
    pub examined: &'context [Examined],
    pub read: &'context ReadContent,
    pub model: &'context ModelCall,
}
pub(super) struct State {
    additional: Vec<TracePart>,
    navigation: Option<ExecutionContent>,
    reads: Vec<Decision>,
    observation_page: usize,
    catalog_page: usize,
    feedback_page: usize,
    stalled: bool,
}
struct Prepared {
    supporting: Vec<Observation>,
    total_observations: usize,
    observation_pages: usize,
    catalog: Vec<Examined>,
    catalog_pages: usize,
    evidence: Vec<TracePart>,
}
pub(super) async fn run(context: Context<'_>) -> Result<Investigation, AnalysisError> {
    let feedback = feedback_pages(context.claim, Some(&context.candidate.check_id))?;
    let relevant = relevant(context.candidate, context.examined);
    let mut state = State {
        additional: Vec::new(),
        navigation: None,
        reads: Vec::new(),
        observation_page: 0,
        catalog_page: 0,
        feedback_page: 0,
        stalled: false,
    };
    loop {
        if let Some(result) = step(&context, &relevant, &feedback, &mut state).await? {
            return Ok(result);
        }
    }
}

async fn step(
    context: &Context<'_>,
    relevant: &[&Examined],
    feedback: &[Vec<FeedbackRow>],
    state: &mut State,
) -> Result<Option<Investigation>, AnalysisError> {
    let prepared = prepare(context, relevant, state)?;
    let Some(decision) = ask(context, state, feedback, &prepared).await? else {
        return Ok(Some(inconclusive(prepared.evidence)));
    };
    let store = trace_store()?;
    store.add_reads(&state.additional)?;
    if let Some(finding) = validation::valid_finding(
        context.claim,
        context.candidate,
        &decision,
        &prepared.evidence,
        &store,
    )? {
        return Ok(Some(Investigation {
            finding: Some(finding),
            parts: prepared.evidence,
        }));
    }
    if state.stalled || !is_navigation(decision.action) {
        return Ok(Some(inconclusive(prepared.evidence)));
    }
    if !valid_page(&decision, &prepared, feedback) || repeated(&state.reads, &decision) {
        state.stalled = true;
        return Ok(None);
    }
    navigation::advance(context, state, decision).await?;
    Ok(None)
}

async fn ask(
    context: &Context<'_>,
    state: &State,
    feedback: &[Vec<FeedbackRow>],
    prepared: &Prepared,
) -> Result<Option<Decision>, AnalysisError> {
    let request = prompt::Context {
        claim: context.claim,
        candidate: context.candidate,
        supporting: &prepared.supporting,
        total_observations: prepared.total_observations,
        observation_page: state.observation_page,
        observation_pages: prepared.observation_pages,
        catalog: &prepared.catalog,
        catalog_page: state.catalog_page,
        catalog_pages: prepared.catalog_pages,
        feedback,
        feedback_page: state.feedback_page,
        evidence: &prepared.evidence,
        completed_read_count: state.reads.len(),
        last_read: state.reads.last(),
        navigation: state.navigation.as_ref(),
        must_decide: state.stalled,
    };
    let prompt = prompt::build(&request)?;
    if prompt.chars().count() > PROMPT_LIMIT {
        return Ok(None);
    }
    let steps = if state.stalled { 1 } else { 2 };
    let decision = investigation_decision(
        ModelRequest {
            prompt,
            purpose: Purpose::Investigate,
        },
        context.model,
        steps,
    )
    .await?;
    Ok(Some(decision))
}

fn relevant<'examined>(
    candidate: &Candidate,
    examined: &'examined [Examined],
) -> Vec<&'examined Examined> {
    let filtered: Vec<&Examined> = examined
        .iter()
        .filter(|item| candidate.execution_ids.contains(&item.execution.id))
        .collect();
    debug_assert!(filtered.len() <= examined.len());
    filtered
}

const fn inconclusive(parts: Vec<TracePart>) -> Investigation {
    let finding = None;
    Investigation { finding, parts }
}

const fn is_navigation(action: DecisionAction) -> bool {
    let navigation = matches!(
        action,
        DecisionAction::Read
            | DecisionAction::Observations
            | DecisionAction::Catalog
            | DecisionAction::Feedback
    );
    navigation
}

fn valid_page(decision: &Decision, prepared: &Prepared, feedback: &[Vec<FeedbackRow>]) -> bool {
    let page = usize::try_from(decision.page).ok();
    match decision.action {
        DecisionAction::Observations => {
            page.is_some_and(|value| value < prepared.observation_pages)
        }
        DecisionAction::Catalog => page.is_some_and(|value| value < prepared.catalog_pages),
        DecisionAction::Feedback => page.is_some_and(|value| value < feedback.len()),
        DecisionAction::Read => true,
        DecisionAction::Submit | DecisionAction::Inconclusive => false,
    }
}

fn repeated(reads: &[Decision], decision: &Decision) -> bool {
    let key = (
        decision.action,
        decision.execution_id.as_deref(),
        decision.cursor.as_str(),
        decision.offset,
        decision.page,
    );
    reads.iter().any(|prior| {
        (
            prior.action,
            prior.execution_id.as_deref(),
            prior.cursor.as_str(),
            prior.offset,
            prior.page,
        ) == key
    })
}

fn prepare(
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
