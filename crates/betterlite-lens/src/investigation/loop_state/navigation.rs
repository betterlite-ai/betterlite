use super::super::loop_state::{Context, State};
use crate::analysis::{AnalysisError, Decision, DecisionAction};
use crate::models::execution::TracePart;

pub(super) enum Outcome {
    Continue,
    Terminal(Vec<TracePart>),
}

pub(super) async fn advance(
    context: &Context<'_>,
    state: &mut State,
    decision: Decision,
) -> Result<Outcome, AnalysisError> {
    state.reads.push(decision.clone());
    match decision.action {
        DecisionAction::Observations => {
            state.observation_page = usize::try_from(decision.page).unwrap_or(usize::MAX);
        }
        DecisionAction::Catalog => {
            state.catalog_page = usize::try_from(decision.page).unwrap_or(usize::MAX);
        }
        DecisionAction::Feedback => {
            state.feedback_page = usize::try_from(decision.page).unwrap_or(usize::MAX);
        }
        DecisionAction::Read => return read_page(context, state, decision).await,
        DecisionAction::Submit | DecisionAction::Inconclusive => {}
    }
    Ok(Outcome::Continue)
}

async fn read_page(
    context: &Context<'_>,
    state: &mut State,
    decision: Decision,
) -> Result<Outcome, AnalysisError> {
    let known = decision.execution_id.as_ref().is_some_and(|execution_id| {
        context
            .examined
            .iter()
            .any(|item| item.execution.id == *execution_id)
    });
    if !known {
        return Ok(Outcome::Terminal(state.additional.clone()));
    }
    let execution_id = decision.execution_id.unwrap_or_default();
    let page = (context.read)(execution_id, decision.cursor, decision.offset).await?;
    if !page
        .parts
        .iter()
        .any(|part| !part.content.is_empty() && !state.additional.contains(part))
    {
        state.stalled = true;
    }
    state.additional.clone_from(&page.parts);
    state.navigation = Some(page);
    Ok(Outcome::Continue)
}
