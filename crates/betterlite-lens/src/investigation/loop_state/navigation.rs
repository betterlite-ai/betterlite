use super::super::loop_state::{Context, State};
use crate::analysis::{AnalysisError, Decision, DecisionAction};

pub(super) async fn advance(
    context: &Context<'_>,
    state: &mut State,
    decision: Decision,
) -> Result<(), AnalysisError> {
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
        DecisionAction::Read => read_page(context, state, decision).await?,
        DecisionAction::Submit | DecisionAction::Inconclusive => {}
    }
    Ok(())
}

async fn read_page(
    context: &Context<'_>,
    state: &mut State,
    decision: Decision,
) -> Result<(), AnalysisError> {
    let Some(execution_id) = decision.execution_id else {
        state.stalled = true;
        return Ok(());
    };
    if !context
        .examined
        .iter()
        .any(|item| item.execution.id == execution_id)
    {
        state.stalled = true;
        return Ok(());
    }
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
    Ok(())
}
