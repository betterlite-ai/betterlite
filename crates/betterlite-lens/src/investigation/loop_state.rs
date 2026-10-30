use super::{PROMPT_LIMIT, investigation_decision};
use super::{prompt, validation};
use crate::analysis::primitives::{FeedbackRow, feedback_pages};
use crate::analysis::{
    AnalysisError, Candidate, Decision, DecisionAction, Examined, Investigation, ModelCall,
    ReadContent,
};
use crate::models::execution::{ExecutionContent, TracePart};
use crate::models::messages::{Claim, ModelRequest, Purpose};
use crate::trace_store::{TraceStore, trace_store};
use preparation::{Prepared, prepare};
mod navigation;
mod preparation;

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
pub(super) async fn run(context: Context<'_>) -> Result<Investigation, AnalysisError> {
    let feedback = feedback_pages(context.claim, Some(&context.candidate.check_id))?;
    let relevant = relevant(context.candidate, context.examined);
    let mut store = trace_store()?;
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
        let (updated_store, outcome) =
            step(&context, &relevant, &feedback, store, &mut state).await?;
        store = updated_store;
        if let Some(result) = outcome {
            return Ok(result);
        }
    }
}

async fn step(
    context: &Context<'_>,
    relevant: &[&Examined],
    feedback: &[Vec<FeedbackRow>],
    store: TraceStore,
    state: &mut State,
) -> Result<(TraceStore, Option<Investigation>), AnalysisError> {
    let prepared = prepare(context, relevant, state)?;
    let Some(decision) = ask(context, state, feedback, &prepared).await? else {
        return Ok((store, Some(inconclusive(prepared.evidence))));
    };
    store.add_reads(&state.additional)?;
    if let Some(finding) = validation::valid_finding(
        context.claim,
        context.candidate,
        &decision,
        &prepared.evidence,
        &store,
    )? {
        return Ok((
            store,
            Some(Investigation {
                finding: Some(finding),
                parts: prepared.evidence,
            }),
        ));
    }
    if state.stalled || !is_navigation(decision.action) {
        return Ok((store, Some(inconclusive(prepared.evidence))));
    }
    if !valid_page(&decision, &prepared, feedback) || repeated(&state.reads, &decision) {
        state.stalled = true;
        return Ok((store, None));
    }
    match navigation::advance(context, state, decision).await? {
        navigation::Outcome::Continue => Ok((store, None)),
        navigation::Outcome::Terminal(parts) => Ok((store, Some(inconclusive(parts)))),
    }
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
